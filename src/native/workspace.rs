//! Board-first presentation. Fixture content is separate from real dispatch.

use std::cell::Cell;

use rocco_contracts::ids::{CommandId, ContentDigest, RunId, TenantId, WorkspaceId};
use rocco_contracts::state::RunState;
use rocco_contracts::workspace::{
    ContinueCoordinator, ContinueOutcome, ContinueRequest, MissingMooAdapter, WorkBoard, WorkItem,
    WorkVersion, WorkspaceScope,
};

use super::{
    App, BASE, CharAttribute, CharFlags, Character, Color, ERR, EventProcessStatus, Focus,
    InputApp, Key, KeyCode, KeyModifier, MI, MUTED, MouseEvent, Native, Row, Surface, WARN, Work,
    allocation_width, draw_mascot, index_coordinate, part, terminal_dimension,
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
                "ROCCO",
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
                "ROCCO",
                "Calmer project home",
                "Ready for a first pass.",
                "Continue the design",
                RunState::Draft,
            ),
            (
                "Recently done",
                "ROCCO",
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
    let index = i32::try_from(index).unwrap_or(i32::MAX);
    RAIL_ITEM_Y.saturating_add(index.saturating_mul(RAIL_ITEM_HEIGHT))
}

/// Paint a rectangle clipped to the owned surface, including extreme coordinates.
fn fill(surface: &mut Surface, x: i32, y: i32, width: i32, height: i32, background: Color) {
    if width <= 0 || height <= 0 {
        return;
    }
    let right = x
        .saturating_add(width)
        .min(i32::try_from(surface.size().width).unwrap_or(i32::MAX));
    let bottom = y
        .saturating_add(height)
        .min(i32::try_from(surface.size().height).unwrap_or(i32::MAX));
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
            let columns = usize::try_from(width).unwrap_or(0);
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
        &"─".repeat(usize::try_from(width.max(0)).unwrap_or(0)),
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
                    &"─".repeat(usize::try_from(width.saturating_sub(1)).unwrap_or(0)),
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
        let text = raw
            .strip_prefix('/')
            .map_or_else(|| raw.to_owned(), |body| format!("/{}", body.trim()));
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
                        let end = ui
                            .transcript
                            .iter()
                            .enumerate()
                            .skip(start.saturating_add(1))
                            .find(|&(_, row)| row.starts_message)
                            .map_or(ui.transcript.len(), |(i, _)| i);
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

    /// Render the responsive board with explicit disconnected-demo labels.
    fn board(&self, surface: &mut Surface, width: i32, height: i32) {
        let rail: i32 = if width >= 92 { 27 } else { 0 };
        if rail > 0 {
            self.sidebar(surface, height);
        }
        let x = rail.saturating_add(2);
        let right = if width >= 118 { 26 } else { 2 };
        let body_width = width.saturating_sub(x).saturating_sub(right);
        let body_background = SHELL;
        ink!(
            surface,
            x,
            3,
            body_width,
            "Workspace",
            INK,
            body_background,
            true,
        );
        ink!(
            surface,
            x,
            4,
            body_width,
            "See the decision that needs you. Everything else stays quiet.",
            SOFT,
            body_background,
            false,
        );
        if height < 20 {
            for (index, card) in self.cards.iter().enumerate() {
                ink!(
                    surface,
                    x,
                    5_i32.saturating_add(index_coordinate(index)),
                    body_width,
                    &format!(
                        "{} {} · {}",
                        if self.selected == index { "›" } else { " " },
                        index.saturating_add(1),
                        card.title
                    ),
                    INK,
                    body_background,
                    self.selected == index,
                );
            }
            ink!(
                surface,
                x,
                height.saturating_sub(1),
                body_width,
                "1–4 open + Continue · t truth · q quit",
                SOFT,
                body_background,
                false,
            );
            return;
        }
        let columns: i32 = if body_width >= 60 { 2 } else { 1 };
        let gutters = columns.saturating_sub(1).saturating_mul(2);
        let card_width = body_width.saturating_sub(gutters).div_euclid(columns);
        let card_height: i32 = 7;
        for (index, card) in self.cards.iter().enumerate() {
            let coordinate = index_coordinate(index);
            let column = coordinate.rem_euclid(columns);
            let row = coordinate.div_euclid(columns);
            let card_x = x.saturating_add(column.saturating_mul(card_width.saturating_add(2)));
            let card_y = 6_i32.saturating_add(row.saturating_mul(card_height.saturating_add(1)));
            if card_y.saturating_add(card_height) >= height.saturating_sub(4) {
                continue;
            }
            let selected = index == self.selected;
            let background = if selected { FOCUS_PANEL } else { PANEL };
            let border = if selected { FOCUS } else { SOFT };
            shell_card!(
                surface,
                card_x,
                card_y,
                card_width,
                card_height,
                &format!("{:02} / {}", index.saturating_add(1), card.lane),
                background,
                border,
            );
            ink!(
                surface,
                card_x.saturating_add(2),
                card_y.saturating_add(2),
                card_width.saturating_sub(4),
                card.title,
                INK,
                background,
                true,
            );
            ink!(
                surface,
                card_x.saturating_add(2),
                card_y.saturating_add(3),
                card_width.saturating_sub(4),
                card.summary,
                SOFT,
                background,
                false,
            );
            ink!(
                surface,
                card_x.saturating_add(2),
                card_y.saturating_add(5),
                card_width.saturating_sub(4),
                card.next,
                INK,
                background,
                false,
            );
            if selected {
                ink!(
                    surface,
                    card_x.saturating_add(card_width).saturating_sub(4),
                    card_y.saturating_add(1),
                    2,
                    self.motion_mark(),
                    INK,
                    background,
                    true,
                );
            }
        }
        if width >= 118 {
            let system_x = width.saturating_sub(24);
            shell_card!(
                surface,
                system_x,
                3,
                22,
                10,
                "ROCCO / SYSTEM",
                SYSTEM_PANEL,
                SOFT,
            );
            ink!(
                surface,
                system_x.saturating_add(2),
                5,
                18,
                "ATTENTION STACK",
                INK,
                SYSTEM_PANEL,
                true,
            );
            ink!(
                surface,
                system_x.saturating_add(2),
                7,
                18,
                "Moo",
                SOFT,
                SYSTEM_PANEL,
                true,
            );
            ink!(
                surface,
                system_x.saturating_add(8),
                7,
                11,
                "offline",
                WARN,
                SYSTEM_PANEL,
                false,
            );
            ink!(
                surface,
                system_x.saturating_add(2),
                8,
                18,
                "AI",
                SOFT,
                SYSTEM_PANEL,
                true
            );
            ink!(
                surface,
                system_x.saturating_add(8),
                8,
                11,
                "offline",
                WARN,
                SYSTEM_PANEL,
                false,
            );
            ink!(
                surface,
                system_x.saturating_add(2),
                10,
                18,
                "Notes: session only",
                SOFT,
                SYSTEM_PANEL,
                false,
            );
            ink!(
                surface,
                system_x.saturating_add(2),
                11,
                18,
                "Open truth [t]",
                INK,
                SYSTEM_PANEL,
                true,
            );
        }
        let footer_y = height.saturating_sub(3);
        line(
            surface,
            x,
            footer_y.saturating_sub(1),
            body_width,
            SOFT,
            SHELL,
        );
        ink!(
            surface,
            x,
            footer_y,
            body_width,
            &format!(
                "FOCUS  {}  ·  Enter opens + requests Continue",
                self.cards[self.selected].title
            ),
            FOCUS,
            SHELL,
            true,
        );
        ink!(
            surface,
            x,
            footer_y.saturating_add(1),
            body_width,
            "← → / Tab select   1–4 open   t truth   q quit",
            SOFT,
            SHELL,
            false,
        );
    }

    /// Explain which displayed facts are synthetic and which integrations are absent.
    fn truth_view(&self, surface: &mut Surface, width: i32, height: i32) {
        let rail: i32 = if width >= 92 { 27 } else { 0 };
        if rail > 0 {
            self.sidebar(surface, height);
        }
        let x = rail.saturating_add(2);
        let body_width = width.saturating_sub(x).saturating_sub(2);
        shell_card!(
            surface,
            x,
            3,
            body_width,
            height.saturating_sub(6),
            "TRUTH / DEMO BOUNDARY",
            SHELL,
            SOFT,
        );
        let lines = [
            "Where this demo tells a story",
            "SYNTHETIC DATA",
            "Board status, projects, activity, diffs and agents are fixtures.",
            "A card saying In progress does NOT mean a process is running.",
            "REAL BEHAVIOR",
            "Selection calls the typed Continue coordinator. Moo is disconnected.",
            "No task starts. No AI replies. Notes are unsent and session-only.",
            "PATH TO LIVE",
            "Moo supplies authenticated workspace snapshots and atomic admission.",
            "Its receipt must bind tenant, work, baseline, revision and fence.",
            "Model transport and Logical-DB durability are still missing.",
            "Fixture hold release: /fixture-continue (never real execution).",
            "ADRs and remaining gates: docs/workspace-experience.md",
        ];
        let wrapped: Vec<String> = lines
            .iter()
            .flat_map(|line| wrap(line, allocation_width(width.saturating_sub(4))))
            .collect();
        let scroll = self.truth_scroll.min(
            wrapped
                .len()
                .saturating_sub(allocation_width(height.saturating_sub(5))),
        );
        for (i, line) in wrapped
            .iter()
            .skip(scroll)
            .take(allocation_width(height.saturating_sub(5)))
            .enumerate()
        {
            ink!(
                surface,
                x.saturating_add(2),
                index_coordinate(i).saturating_add(5),
                body_width.saturating_sub(4),
                line,
                INK,
                SHELL,
                i == 0 || i == 1 || i == 4 || i == 7,
            );
        }
        ink!(
            surface,
            x.saturating_add(2),
            height.saturating_sub(2),
            body_width.saturating_sub(4),
            "↑ ↓ scroll · Esc / t Back · no state changed",
            FOCUS,
            SHELL,
            true,
        );
        if width >= 120 && height >= 32 {
            draw_mascot(surface, width.saturating_sub(28), height.saturating_sub(15));
        }
    }

    /// Render the selected session, unsent input and fixture inspection context.
    fn work(&self, surface: &mut Surface, width: i32, height: i32) {
        let card = &self.cards[self.selected];
        let ui = &card.session;
        let rail: i32 = if width >= 110 { 27 } else { 0 };
        if rail > 0 {
            self.sidebar(surface, height);
        }
        let x = rail.saturating_add(2);
        let has_system = width >= 112;
        let right_width = if has_system { 29 } else { 0 };
        let main_width = width
            .saturating_sub(x)
            .saturating_sub(right_width)
            .saturating_sub(2);
        let context = !has_system && ui.focus != Focus::Input;
        ink!(
            surface,
            x,
            3,
            main_width,
            &format!("HOME  /  {}  /  {}", card.project, card.title),
            INK,
            SHELL,
            true,
        );
        ink!(
            surface,
            x,
            4,
            main_width,
            &card.dispatch,
            WARN,
            SHELL,
            false,
        );
        shell_card!(
            surface,
            x,
            6,
            main_width,
            height.saturating_sub(12).max(5),
            if context {
                "CONTEXT / DEMO DATA"
            } else {
                "HUMAN + AI"
            },
            SHELL,
            SOFT,
        );
        let rows = if ui.help {
            [
                "/home · return to the board",
                "/continue · request Moo continuation",
                "/truth · what is synthetic and why",
                "/next · reveal a fixture change",
                "/inspect · show its anchored diff",
                "/feedback TEXT · hold fixture work",
                "/fixture-continue · release ONLY the fixture",
                "/files /agents /changes · context",
                "Tab · switch input/context",
                "F1 · close this guide",
                "Esc · Home, draft retained",
            ]
            .into_iter()
            .map(|text| Row::plain(text, BASE))
            .collect()
        } else if context {
            match ui.focus {
                Focus::Files => ui.files_rows(),
                Focus::Agents => ui.agents_rows(),
                _ if ui.inspection && ui.demo.selected.is_some() => ui.inspection_rows(),
                _ => ui.changes_rows(),
            }
        } else {
            ui.transcript.iter().map(|line| line.row.clone()).collect()
        };
        let space = allocation_width(height.saturating_sub(15));
        let start = if context {
            0
        } else {
            rows.len().saturating_sub(space).saturating_sub(ui.scroll)
        };
        for (i, row) in rows.iter().skip(start).take(space).enumerate() {
            // Authored fixture rows were designed for the old panel. Reflow
            // independently from its width assertions; text still clips visibly.
            let text: String = row.parts.iter().map(|part| part.text.as_str()).collect();
            ink!(
                surface,
                x.saturating_add(2),
                8_i32.saturating_add(index_coordinate(i)),
                main_width.saturating_sub(4),
                &text,
                INK,
                SHELL,
                false,
            );
        }
        if has_system {
            let system_x = width.saturating_sub(right_width);
            shell_card!(
                surface,
                system_x,
                6,
                right_width.saturating_sub(2),
                8,
                "ROCCO / SYSTEM",
                SYSTEM_PANEL,
                SOFT,
            );
            let (priority, detail) = match ui.demo.work {
                Work::Held { .. } => ("Feedback held · DEMO", "Your anchor stays selected"),
                Work::Ready if ui.demo.cursor > 0 => {
                    ("Change ready · DEMO", "Inspect before continuing")
                }
                Work::Ready => ("Moo connection needed", "No work has started"),
            };
            ink!(
                surface,
                system_x.saturating_add(2),
                8,
                right_width.saturating_sub(6),
                priority,
                WARN,
                SYSTEM_PANEL,
                true,
            );
            ink!(
                surface,
                system_x.saturating_add(2),
                10,
                right_width.saturating_sub(6),
                detail,
                SOFT,
                SYSTEM_PANEL,
                false,
            );
            ink!(
                surface,
                system_x.saturating_add(2),
                12,
                right_width.saturating_sub(6),
                "No model. No execution.",
                INK,
                SYSTEM_PANEL,
                true,
            );
            if height >= 26 {
                let context_y = 16;
                let context_h = height.saturating_sub(22).max(5);
                shell_card!(
                    surface,
                    system_x,
                    context_y,
                    right_width.saturating_sub(2),
                    context_h,
                    match ui.focus {
                        Focus::Files => "FILES / fixture",
                        Focus::Agents => "AGENTS / fixture",
                        _ => "CHANGES / fixture",
                    },
                    PANEL,
                    SOFT,
                );
                let context_rows = match ui.focus {
                    Focus::Files => ui.files_rows(),
                    Focus::Agents => ui.agents_rows(),
                    _ if ui.inspection && ui.demo.selected.is_some() => ui.inspection_rows(),
                    _ => ui.changes_rows(),
                };
                for (i, row) in context_rows
                    .iter()
                    .take(allocation_width(context_h.saturating_sub(2)))
                    .enumerate()
                {
                    let text: String = row.parts.iter().map(|part| part.text.as_str()).collect();
                    ink!(
                        surface,
                        system_x.saturating_add(2),
                        context_y
                            .saturating_add(1)
                            .saturating_add(index_coordinate(i)),
                        right_width.saturating_sub(6),
                        &text,
                        SOFT,
                        PANEL,
                        false,
                    );
                }
            }
        }
        fill(surface, x, height.saturating_sub(5), main_width, 2, RAIL);
        let draft = if ui.input.is_empty() {
            "Leave a note… (unsent; AI disconnected)"
        } else {
            &ui.input
        };
        ink!(
            surface,
            x.saturating_add(1),
            height.saturating_sub(4),
            main_width.saturating_sub(2),
            &format!("❯ {draft}"),
            INK,
            RAIL,
            false,
        );
        ink!(
            surface,
            x,
            height.saturating_sub(2),
            main_width,
            "Esc Home  ·  Tab context  ·  /continue  /next  /inspect  /truth",
            SOFT,
            SHELL,
            false,
        );
        ink!(
            surface,
            x,
            height.saturating_sub(1),
            main_width,
            "DEMO DATA · session-only notes · no model · no execution",
            WARN,
            SHELL,
            false,
        );
    }
}

impl InputApp for Workspace {
    fn on_key_event(&mut self, key: Key, ch: char) -> EventProcessStatus {
        if key.modifier == KeyModifier::Ctrl && matches!(key.code, KeyCode::C | KeyCode::D) {
            App::close();
            return EventProcessStatus::Processed;
        }
        let was_selected = self.selected;
        if self.truth {
            if key.code == KeyCode::Escape || ch == 't' {
                self.truth = false;
            } else if key.code == KeyCode::Down {
                self.truth_scroll = self.truth_scroll.saturating_add(1).min(24);
            } else if key.code == KeyCode::Up {
                self.truth_scroll = self.truth_scroll.saturating_sub(1);
            }
        } else if !self.entered {
            match key.code {
                KeyCode::Right | KeyCode::Down | KeyCode::Tab => {
                    self.selected = self.selected.saturating_add(1).rem_euclid(self.cards.len())
                }
                KeyCode::Left | KeyCode::Up => {
                    self.selected = self
                        .selected
                        .saturating_add(self.cards.len())
                        .saturating_sub(1)
                        .rem_euclid(self.cards.len())
                }
                KeyCode::Enter => self.enter(),
                _ => match ch {
                    '1'..='4' => {
                        if let Some(digit) = ch.to_digit(10)
                            && let Ok(selected) = usize::try_from(digit.saturating_sub(1))
                        {
                            self.selected = selected;
                            self.enter();
                        }
                    }
                    't' | '?' => {
                        self.truth = true;
                        self.truth_scroll = 0;
                    }
                    'q' => App::close(),
                    _ => {}
                },
            }
        } else {
            let ui = &mut self.cards[self.selected].session;
            if key.code == KeyCode::Escape {
                self.entered = false;
            } else if key.code == KeyCode::F6 {
                ui.inspection = !ui.inspection;
                ui.focus = Focus::Changes;
            } else if key.code == KeyCode::F5
                || (ch == 'c' && ui.focus != Focus::Input && !ui.command_mode)
            {
                self.continue_selected();
            } else if key.code == KeyCode::Enter && (ui.focus == Focus::Input || ui.command_mode) {
                self.submit();
            } else if matches!(key.code, KeyCode::Left | KeyCode::Right) && ui.focus == Focus::Input
            {
                // The editor is append-only; arrows must not steal input focus.
            } else {
                ui.on_key_event(key, ch);
            }
        }
        if self.selected != was_selected || self.entered {
            self.start_motion();
        }
        EventProcessStatus::Processed
    }

    fn on_mouse_event(&mut self, event: &MouseEvent) -> EventProcessStatus {
        if let &MouseEvent::Pressed(data) = event {
            let (width, height) = self.size.get();
            if data.y <= 1 && data.x >= width.saturating_sub(22) {
                self.truth = !self.truth;
            } else if self.entered && data.y == 2 && data.x < 8 {
                self.entered = false;
            } else if self.entered && !self.truth && data.y >= height.saturating_sub(4) {
                self.cards[self.selected].session.focus = Focus::Input;
            } else if !self.entered && !self.truth && height < 20 && (5..9).contains(&data.y) {
                if let Ok(selected) = usize::try_from(data.y.saturating_sub(5)) {
                    self.selected = selected;
                }
                self.enter();
            } else if !self.entered
                && !self.truth
                && width >= 92
                && data.x >= RAIL_ITEM_X
                && data.x < RAIL_ITEM_X.saturating_add(RAIL_ITEM_WIDTH)
                && data.y >= RAIL_ITEM_Y
            {
                if let Ok(selected) = usize::try_from(
                    data.y
                        .saturating_sub(RAIL_ITEM_Y)
                        .div_euclid(RAIL_ITEM_HEIGHT),
                ) && selected < self.cards.len()
                    && data.y < rail_item_y(selected).saturating_add(RAIL_ITEM_HEIGHT)
                {
                    self.selected = selected;
                    self.enter();
                }
            } else if !self.entered && !self.truth && data.y >= 6 && data.y < 21 {
                let rail: i32 = if width >= 92 { 27 } else { 0 };
                let x = rail.saturating_add(2);
                let right = if width >= 118 { 26 } else { 2 };
                let body_width = width.saturating_sub(x).saturating_sub(right);
                let count: i32 = if body_width >= 60 { 2 } else { 1 };
                let card_width = body_width
                    .saturating_sub(count.saturating_sub(1).saturating_mul(2))
                    .div_euclid(count);
                let offset = data
                    .x
                    .saturating_sub(x)
                    .div_euclid(card_width.saturating_add(2).max(1));
                let row = data.y.saturating_sub(6).div_euclid(8);
                let selected = usize::try_from(row).ok().and_then(|row_index| {
                    usize::try_from(count).ok().and_then(|column_count| {
                        usize::try_from(offset).ok().map(|column| {
                            row_index
                                .saturating_mul(column_count)
                                .saturating_add(column)
                        })
                    })
                });
                if data.x >= x
                    && selected.is_some_and(|card| card < self.cards.len())
                    && offset >= 0
                    && offset < count
                    && data.y.saturating_sub(6).rem_euclid(8) < 7
                    && let Some(selected) = selected
                {
                    self.selected = selected;
                    self.enter();
                }
            }
        }
        EventProcessStatus::Processed
    }

    fn on_paint(&self, surface: &mut Surface) {
        let width = terminal_dimension(surface.size().width);
        let height = terminal_dimension(surface.size().height);
        self.size.set((width, height));
        surface.reset(Character::new(' ', INK, SHELL, CharFlags::None));
        if width < 50 || height < 12 {
            ink!(
                surface,
                0,
                0,
                width,
                "Rocco · resize to 50x12 · Ctrl-C exits",
                WARN,
                SHELL,
                true,
            );
            return;
        }
        fill(surface, 0, 0, width, 2, RAIL);
        ink!(
            surface,
            2,
            0,
            width.saturating_sub(26),
            "◉  ROCCO  /  WORKSPACE",
            INK,
            RAIL,
            true,
        );
        ink!(
            surface,
            2,
            1,
            width.saturating_sub(26),
            "You, the system, and an AI — in one place.",
            SOFT,
            RAIL,
            false,
        );
        line(surface, 0, 2, width, SOFT, SHELL);
        ink!(
            surface,
            width.saturating_sub(23),
            0,
            21,
            "DEMO DATA  [truth ↗]",
            WARN,
            RAIL,
            false,
        );
        if let Some(error) = self.setup_error.as_ref() {
            ink!(
                surface,
                2,
                2,
                width.saturating_sub(4),
                &format!("Fixture setup refused: {error}"),
                ERR,
                SHELL,
                true,
            );
        } else if self.truth {
            self.truth_view(surface, width, height);
        } else if self.entered {
            self.work(surface, width, height);
        } else {
            self.board(surface, width, height);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lgwks_deps::appcui::prelude::{MouseButton, MouseEventData};

    #[test]
    fn rectangle_fill_clips_extreme_bounds_and_preserves_empty_regions() -> std::io::Result<()> {
        let mut surface = Surface::new(2, 2);
        surface.reset(Character::new(' ', INK, SHELL, CharFlags::None));
        fill(&mut surface, 0, 0, i32::MAX, i32::MAX, FOCUS_PANEL);
        for y in 0..2 {
            for x in 0..2 {
                assert_eq!(scene_cell(&surface, x, y)?.background, FOCUS_PANEL);
            }
        }
        for (x, y, width, height) in [
            (0, 0, 0, 2),
            (0, 0, 2, -1),
            (i32::MAX, i32::MAX, 2, 2),
            (i32::MIN, i32::MIN, i32::MAX, i32::MAX),
        ] {
            fill(&mut surface, x, y, width, height, SHELL);
        }
        for y in 0..2 {
            for x in 0..2 {
                assert_eq!(scene_cell(&surface, x, y)?.background, FOCUS_PANEL);
            }
        }
        Ok(())
    }

    fn paint(ui: &Workspace, width: u32, height: u32) -> String {
        let surface = scene(ui, width, height);
        cells(&surface, width, height)
    }

    fn scene(ui: &Workspace, width: u32, height: u32) -> Surface {
        let mut surface = Surface::new(width, height);
        ui.on_paint(&mut surface);
        surface
    }

    fn cells(surface: &Surface, width: u32, height: u32) -> String {
        let terminal_height = terminal_dimension(height);
        let terminal_width = terminal_dimension(width);
        (0..terminal_height)
            .map(|y| {
                let mut row = (0..terminal_width)
                    .map(|x| surface.char(x, y).map_or(' ', |cell| cell.code))
                    .collect::<String>();
                row.push('\n');
                row
            })
            .collect()
    }

    fn svg_color(color: Color) -> &'static str {
        match color {
            Color::Black => "#0b1020",
            Color::DarkBlue => "#182449",
            Color::DarkGreen => "#17483a",
            Color::Teal => "#173d4a",
            Color::DarkRed => "#5b1c32",
            Color::Magenta => "#43205e",
            Color::Olive => "#62521e",
            Color::Silver => "#c6cedb",
            Color::Gray => "#74809a",
            Color::Blue => "#75a7ff",
            Color::Green => "#65e1ae",
            Color::Aqua => "#66d9ff",
            Color::Red => "#ff6b96",
            Color::Pink => "#e496ff",
            Color::Yellow => "#ffd166",
            Color::White => "#f7fbff",
            Color::Transparent => "#0b1020",
        }
    }

    fn svg_escape(value: &str) -> String {
        value
            .replace('&', "&amp;")
            .replace('<', "&lt;")
            .replace('>', "&gt;")
    }

    fn scene_cell(surface: &Surface, x: i32, y: i32) -> std::io::Result<&Character> {
        surface.char(x, y).ok_or_else(|| {
            std::io::Error::other(format!("rendered scene has no cell at ({x}, {y})"))
        })
    }

    fn write_scene_svg(
        file: &mut std::fs::File,
        surface: &Surface,
        width: u32,
        height: u32,
    ) -> std::io::Result<()> {
        use std::io::Write;
        const CELL_W: i32 = 9;
        const CELL_H: i32 = 18;
        writeln!(
            file,
            "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"1120\" height=\"650\" viewBox=\"0 0 1120 650\"><rect width=\"1120\" height=\"650\" fill=\"#0b1020\"/>"
        )?;
        let terminal_height = terminal_dimension(height);
        let terminal_width = terminal_dimension(width);
        for y in 0..terminal_height {
            let mut start = 0;
            while start < terminal_width {
                let background = scene_cell(surface, start, y)?.background;
                let mut end = start.saturating_add(1);
                while end < terminal_width && scene_cell(surface, end, y)?.background == background
                {
                    end = end.saturating_add(1);
                }
                writeln!(
                    file,
                    "<rect x=\"{}\" y=\"{}\" width=\"{}\" height=\"{CELL_H}\" fill=\"{}\"/>",
                    16i32.saturating_add(start.saturating_mul(CELL_W)),
                    8i32.saturating_add(y.saturating_mul(CELL_H)),
                    end.saturating_sub(start).saturating_mul(CELL_W),
                    svg_color(background)
                )?;
                start = end;
            }
            let mut x = 0;
            while x < terminal_width {
                let cell = scene_cell(surface, x, y)?;
                let foreground = cell.foreground;
                let background = cell.background;
                let flags = cell.flags;
                let mut text = String::new();
                let mut end = x;
                while end < terminal_width {
                    let next = scene_cell(surface, end, y)?;
                    if next.foreground != foreground
                        || next.background != background
                        || next.flags != flags
                    {
                        break;
                    }
                    text.push(next.code);
                    end = end.saturating_add(1);
                }
                if !text.trim().is_empty() {
                    let weight = if flags == CharFlags::Bold {
                        " font-weight=\"700\""
                    } else {
                        ""
                    };
                    writeln!(
                        file,
                        "<text x=\"{}\" y=\"{}\" fill=\"{}\" font-family=\"Menlo,monospace\" font-size=\"14\" xml:space=\"preserve\"{weight}>{}</text>",
                        16i32.saturating_add(x.saturating_mul(CELL_W)),
                        22i32.saturating_add(y.saturating_mul(CELL_H)),
                        svg_color(foreground),
                        svg_escape(&text)
                    )?;
                }
                x = end;
            }
        }
        writeln!(
            file,
            "<text x=\"16\" y=\"625\" fill=\"#a9b5c9\" font-family=\"sans-serif\" font-size=\"12\">Actual AppCUI cell-render preview; not a Terminal screenshot. ANSI16 palette shown faithfully.</text></svg>"
        )?;
        Ok(())
    }

    fn press(ui: &mut Workspace, code: KeyCode) {
        ui.on_key_event(Key::new(code, KeyModifier::None), '\0');
    }

    fn command(ui: &mut Workspace, text: &str) {
        ui.cards[ui.selected].session.input = text.into();
        ui.cards[ui.selected].session.command_mode = true;
        press(ui, KeyCode::Enter);
    }

    fn click(ui: &mut Workspace, x: i32, y: i32) {
        ui.on_mouse_event(&MouseEvent::Pressed(MouseEventData {
            x,
            y,
            button: MouseButton::Left,
            modifier: KeyModifier::None,
        }));
    }

    #[test]
    fn board_is_the_entry_and_truth_survives_small_layouts() {
        let ui = Workspace::default();
        for (width, height) in [(50, 12), (80, 24), (104, 25), (120, 32), (160, 48)] {
            let text = paint(&ui, width, height);
            assert!(text.contains("Workspace"));
            assert!(text.contains("DEMO DATA"));
            assert!(!text.contains("AI · not connected"));
        }
        assert!(paint(&ui, 120, 32).contains("Needs you"));
        assert!(paint(&ui, 120, 32).contains("Recently done"));
    }

    #[test]
    fn rendered_frames_can_be_reviewed_without_terminal_app_access() -> std::io::Result<()> {
        let mut ui = Workspace::default();
        let home = paint(&ui, 120, 32);
        ui.enter();
        command(&mut ui, "/next");
        command(&mut ui, "/inspect");
        let work = paint(&ui, 120, 32);
        command(&mut ui, "/truth");
        let truth = paint(&ui, 120, 32);
        assert!(home.contains("Workspace"));
        assert!(work.contains("fixture-01"));
        assert!(truth.contains("PATH TO LIVE"));
        // Optional evidence output from the actual cell renderer. This is
        // explicitly NOT a screenshot or a substitute for real PTY journeys.
        if let Some(directory) = std::env::var_os("ROCCO_CAPTURE_DIR") {
            let mut preview = Workspace::default();
            let home_scene = scene(&preview, 120, 32);
            preview.enter();
            command(&mut preview, "/next");
            command(&mut preview, "/inspect");
            let work_scene = scene(&preview, 120, 32);
            command(&mut preview, "/truth");
            let truth_scene = scene(&preview, 120, 32);
            for (name, scene) in [
                ("home", home_scene),
                ("work", work_scene),
                ("truth", truth_scene),
            ] {
                let path =
                    std::path::PathBuf::from(&directory).join(format!("workspace-{name}.svg"));
                let mut file = std::fs::OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(path)?;
                write_scene_svg(&mut file, &scene, 120, 32)?;
            }
        }
        Ok(())
    }

    #[test]
    fn quiet_torque_keeps_depth_neutral_and_focus_singular() {
        let ui = Workspace::default();
        let surface = scene(&ui, 120, 32);
        let mut colors = Vec::with_capacity(120 * 32);
        for y in 0..32 {
            for x in 0..120 {
                let background = surface.char(x, y).map(|cell| cell.background);
                assert!(
                    background.is_some(),
                    "the 120x32 render must own every cell"
                );
                if let Some(background) = background {
                    colors.push(background);
                }
            }
        }

        assert_eq!(colors.len(), 120 * 32);
        assert!(colors.contains(&FOCUS_PANEL));
        assert!(!colors.contains(&Color::Teal));
        assert!(!colors.contains(&Color::Magenta));
        assert!(!colors.contains(&Color::DarkGreen));
        assert_eq!(
            surface.char(30, 7).map(|cell| cell.background),
            Some(FOCUS_PANEL)
        );
        assert_eq!(surface.char(62, 7).map(|cell| cell.background), Some(SHELL));
        assert_eq!(surface.char(97, 4).map(|cell| cell.background), Some(SHELL));
    }

    #[test]
    fn rail_rows_are_visible_targets_for_the_same_continue_path_as_cards() {
        let mut ui = Workspace::default();
        ui.size.set((120, 32));
        click(&mut ui, RAIL_ITEM_X + 3, rail_item_y(2) + 1);

        assert_eq!(ui.selected, 2);
        assert!(ui.entered);
        let selected = scene(&ui, 120, 32);
        assert_eq!(
            selected
                .char(RAIL_ITEM_X + 1, rail_item_y(2))
                .map(|cell| cell.background),
            Some(FOCUS_PANEL)
        );
        assert!(paint(&ui, 120, 32).contains("Awaiting Moo · nothing started"));
    }

    #[test]
    fn selection_and_all_continue_aliases_never_release_a_fixture_hold() {
        let mut ui = Workspace::default();
        press(&mut ui, KeyCode::Enter);
        assert!(paint(&ui, 120, 32).contains("Awaiting Moo · nothing started"));
        command(&mut ui, "/next");
        command(&mut ui, "/feedback keep bounded");
        for text in ["/continue", " / continue ", "/continue unexpected"] {
            command(&mut ui, text);
            assert!(matches!(ui.cards[0].session.demo.work, Work::Held { .. }));
            assert_eq!(
                ui.coordinator
                    .board()
                    .get(ui.cards[0].run)
                    .map(|record| record.state()),
                Some(RunState::AwaitingAuthority)
            );
        }
        press(&mut ui, KeyCode::F5);
        assert!(matches!(ui.cards[0].session.demo.work, Work::Held { .. }));
        command(&mut ui, "/fixture-continue");
        assert_eq!(ui.cards[0].session.demo.work, Work::Ready);
    }

    #[test]
    fn normalized_commands_do_not_fabricate_ai_messages() {
        let mut ui = Workspace::default();
        ui.enter();
        command(&mut ui, "  /next  ");
        assert_eq!(ui.cards[0].session.demo.cursor, 1);
        assert!(!paint(&ui, 120, 32).contains("Message received"));
        command(&mut ui, "a note");
        assert!(paint(&ui, 120, 32).contains("Unsent · no model connected"));
    }

    #[test]
    fn focus_feedback_is_input_driven_and_never_claims_live_progress() {
        let mut ui = Workspace::default();
        press(&mut ui, KeyCode::Right);
        assert_eq!(ui.motion, 6);
        let focused = paint(&ui, 120, 32);
        assert!(focused.contains("DEMO DATA"));
        assert!(focused.contains("offline"));
        // A paint is read-only; focus feedback cannot become an idle process.
        assert_eq!(ui.motion, 6);
    }

    #[test]
    fn drafts_and_inspection_survive_home_and_are_scoped_to_work() {
        let mut ui = Workspace::default();
        ui.enter();
        command(&mut ui, "/next");
        command(&mut ui, "/inspect");
        assert!(paint(&ui, 120, 32).contains("fixture-01"));
        ui.cards[0].session.input = "keep my draft 🦝".into();
        press(&mut ui, KeyCode::Escape);
        press(&mut ui, KeyCode::Right);
        press(&mut ui, KeyCode::Enter);
        assert!(ui.cards[1].session.input.is_empty());
        press(&mut ui, KeyCode::Escape);
        press(&mut ui, KeyCode::Left);
        press(&mut ui, KeyCode::Enter);
        assert_eq!(ui.cards[0].session.input, "keep my draft 🦝");
        assert!(ui.cards[0].session.inspection);
    }

    #[test]
    fn help_and_truth_are_reachable_without_losing_a_draft() {
        let mut ui = Workspace::default();
        ui.enter();
        press(&mut ui, KeyCode::F1);
        assert!(paint(&ui, 120, 40).contains("/next"));
        command(&mut ui, "/truth");
        for _ in 0..24 {
            press(&mut ui, KeyCode::Down);
        }
        assert!(paint(&ui, 50, 12).contains("workspace-experience.md"));
        press(&mut ui, KeyCode::Escape);
        assert!(!ui.truth);
        assert!(ui.entered);
    }

    #[test]
    fn completed_work_refuses_continue_instead_of_restarting() {
        let mut ui = Workspace {
            selected: 3,
            ..Workspace::default()
        };
        ui.enter();
        assert!(ui.cards[3].dispatch.starts_with("Not continued:"));
        assert_eq!(
            ui.coordinator
                .board()
                .get(ui.cards[3].run)
                .map(|record| record.state()),
            Some(RunState::Completed)
        );
    }

    #[test]
    fn notes_are_bounded_and_fresh_process_does_not_claim_durability() {
        let mut ui = Workspace::default();
        ui.enter();
        for index in 0..40 {
            command(&mut ui, &format!("note {index}"));
        }
        assert_eq!(ui.cards[0].session.user_messages, 16);
        assert!(!paint(&ui, 120, 48).contains("You · note 0 "));
        let fresh = Workspace::default();
        assert!(fresh.cards[0].session.input.is_empty());
        assert_eq!(fresh.cards[0].dispatch, "Not requested");
        assert!(fresh.cards[0].session.transcript.iter().any(|line| {
            line.row
                .parts
                .iter()
                .any(|part| part.text.contains("lost on exit"))
        }));
    }
}
