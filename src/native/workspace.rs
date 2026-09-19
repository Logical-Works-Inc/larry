//! Board-first presentation. Fixture content is separate from real dispatch.

use std::cell::Cell;

use rocco_contracts::ids::{CommandId, ContentDigest, RunId, TenantId, WorkspaceId};
use rocco_contracts::state::RunState;
use rocco_contracts::workspace::{
    ContinueCoordinator, ContinueOutcome, ContinueRequest, MissingMooAdapter, WorkBoard, WorkItem,
    WorkVersion, WorkspaceScope,
};

use super::*;

struct Card {
    lane: &'static str,
    project: &'static str,
    title: &'static str,
    summary: &'static str,
    next: &'static str,
    run: RunId,
    command: CommandId,
    session: Native,
    dispatch: String,
}

/// One disposable view. No timer, thread, filesystem write or network client.
pub(super) struct Workspace {
    cards: Vec<Card>,
    selected: usize,
    entered: bool,
    truth: bool,
    truth_scroll: usize,
    coordinator: ContinueCoordinator,
    size: Cell<(i32, i32)>,
    setup_error: Option<String>,
    /// An input-driven focus marker. It never represents background progress
    /// or a live connection, and deliberately has no timer.
    motion: u8,
}

impl Default for Workspace {
    fn default() -> Self {
        // Synthetic identities are NEVER credentials; the missing adapter
        // refuses execution. Live identity must come from the authenticated host.
        let scope = WorkspaceScope {
            tenant: TenantId::from_bytes([1; 16]),
            workspace: WorkspaceId::from_bytes([2; 16]),
        };
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
            .enumerate()
            .map(|(index, (lane, project, title, summary, next, state))| {
                let run = RunId::from_bytes([index as u8 + 3; 16]);
                let version = WorkVersion {
                    revision: 1,
                    baseline: ContentDigest::over(&[title.as_bytes()]),
                    fence: 1,
                };
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
                    command: CommandId::from_bytes([index as u8 + 20; 16]),
                    session,
                    dispatch: "Not requested".into(),
                }
            })
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

// AppCUI arrives through the estate storefront in ANSI16 mode. These are
// deliberate terminal-IDE surfaces, not claims of 24-bit fidelity: a dark
// shell establishes depth; green is focus, magenta is System, amber still
// means synthetic/needs-attention and always has a textual label too.
const SHELL: Color = Color::Black;
const RAIL: Color = Color::DarkBlue;
const PANEL: Color = Color::Teal;
const FOCUS_PANEL: Color = Color::DarkGreen;
const SYSTEM_PANEL: Color = Color::Magenta;
const INK: Color = Color::White;
const SOFT: Color = Color::Silver;

fn fill(surface: &mut Surface, x: i32, y: i32, width: i32, height: i32, background: Color) {
    if width <= 0 || height <= 0 {
        return;
    }
    for row in y.max(0)..(y + height).max(0) {
        for column in x.max(0)..(x + width).max(0) {
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
        let (x, y, width) = ($x, $y, $width);
        let text = $text;
        let foreground = $foreground;
        let background = $background;
        if width > 0 && y >= 0 && x >= 0 {
            let clipped: String = text.chars().take(width as usize).collect();
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
            if text.chars().count() > width as usize {
                $surface.write_char(
                    x + width - 1,
                    y,
                    Character::new('…', SOFT, background, CharFlags::None),
                );
            }
        }
    }};
}

fn line(surface: &mut Surface, x: i32, y: i32, width: i32, color: Color, background: Color) {
    ink!(
        surface,
        x,
        y,
        width,
        &"─".repeat(width.max(0) as usize),
        color,
        background,
        false,
    );
}

macro_rules! shell_card {
    ($surface:expr, $x:expr, $y:expr, $width:expr, $height:expr, $title:expr, $background:expr, $border:expr $(,)?) => {{
        let (x, y, width, height) = ($x, $y, $width, $height);
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
                y + height - 1,
                width,
                "╰",
                border,
                background,
                false
            );
            if width > 1 {
                ink!(
                    $surface,
                    x + 1,
                    y + height - 1,
                    width - 1,
                    &"─".repeat((width - 1) as usize),
                    border,
                    background,
                    false
                );
            }
            for row in y + 1..y + height - 1 {
                ink!($surface, x, row, 1, "│", border, background, false);
                ink!(
                    $surface,
                    x + width - 1,
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

fn wrap(line: &str, width: usize) -> Vec<String> {
    let mut rows = vec![String::new()];
    for word in line.split_whitespace() {
        let last = rows.len() - 1;
        if !rows[last].is_empty() && rows[last].chars().count() + 1 + word.chars().count() > width {
            rows.push(String::new());
        }
        let last = rows.len() - 1;
        if !rows[last].is_empty() {
            rows[last].push(' ');
        }
        for ch in word.chars() {
            if rows.last().is_some_and(|row| row.chars().count() >= width) {
                rows.push(String::new());
            }
            let last = rows.len() - 1;
            rows[last].push(ch);
        }
    }
    rows
}

impl Workspace {
    fn start_motion(&mut self) {
        // Event-driven visual feedback only. A continuous frame loop was
        // measured to exceed the terminal's 1 MiB idle-output bound in under
        // 30 seconds, so it is rejected for this lightweight surface.
        self.motion = 6;
    }

    fn motion_mark(&self) -> &'static str {
        ["▁", "▃", "▅", "▇", "█", "▇", "▅"]
            .get(self.motion as usize)
            .copied()
            .unwrap_or("▏")
    }

    fn sidebar(&self, surface: &mut Surface, height: i32) {
        if height < 12 {
            return;
        }
        fill(surface, 0, 2, 27, height - 2, RAIL);
        ink!(surface, 2, 4, 23, "WORKSPACE", SOFT, RAIL, true);
        ink!(surface, 2, 6, 23, "▣  Home", INK, RAIL, true);
        line(surface, 2, 8, 23, SOFT, RAIL);
        ink!(surface, 2, 10, 23, "WORK IN VIEW", SOFT, RAIL, true);
        for (index, _) in self.cards.iter().enumerate() {
            let y = 12 + index as i32 * 3;
            if y + 1 >= height - 3 {
                break;
            }
            let active = self.selected == index;
            if active {
                fill(surface, 1, y - 1, 25, 2, FOCUS_PANEL);
            }
            ink!(
                surface,
                2,
                y - 1,
                22,
                &format!(
                    "{} {:02}",
                    if active { self.motion_mark() } else { "·" },
                    index + 1
                ),
                if active { INK } else { SOFT },
                if active { FOCUS_PANEL } else { RAIL },
                active,
            );
            ink!(
                surface,
                5,
                y,
                19,
                match index {
                    0 => "Review retry",
                    1 => "Workspace handoff",
                    2 => "Project home",
                    _ => "Bounded input",
                },
                if active { INK } else { SOFT },
                if active { FOCUS_PANEL } else { RAIL },
                active,
            );
        }
        if height >= 28 {
            line(surface, 2, height - 5, 23, SOFT, RAIL);
            ink!(
                surface,
                2,
                height - 3,
                23,
                "⊙ DEMO / disconnected",
                WARN,
                RAIL,
                true,
            );
            ink!(
                surface,
                2,
                height - 2,
                23,
                "t  truth & data source",
                SOFT,
                RAIL,
                false,
            );
        }
    }

    fn continue_selected(&mut self) {
        let card = &self.cards[self.selected];
        let request = ContinueRequest {
            command: card.command,
            scope: self.coordinator.board().scope(),
            run: card.run,
            expected_revision: 1,
        };
        let result = self
            .coordinator
            .continue_work(&mut MissingMooAdapter, request);
        self.cards[self.selected].dispatch = match result {
            Ok(ContinueOutcome::AwaitingDependency) => "Awaiting Moo · nothing started".into(),
            Ok(ContinueOutcome::Queued(_)) => "Owner queued · not completed".into(),
            Err(error) => format!("Not continued: {error}"),
        };
    }

    fn enter(&mut self) {
        self.entered = true;
        self.start_motion();
        self.continue_selected();
    }

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
                ui.user_messages += 1;
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
                            .skip(start + 1)
                            .find(|(_, row)| row.starts_message)
                            .map_or(ui.transcript.len(), |(i, _)| i);
                        ui.transcript.drain(start..end);
                        ui.user_messages -= 1;
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

    fn board(&self, surface: &mut Surface, width: i32, height: i32) {
        let rail = if width >= 92 { 27 } else { 0 };
        if rail > 0 {
            self.sidebar(surface, height);
        }
        let x = rail + 2;
        let right = if width >= 118 { 26 } else { 2 };
        let body_width = width - x - right;
        let body_background = SHELL;
        ink!(
            surface,
            x,
            3,
            body_width,
            "Your workspace",
            INK,
            body_background,
            true,
        );
        ink!(
            surface,
            x,
            4,
            body_width,
            "A live-looking workspace should not make you guess what is live.",
            SOFT,
            body_background,
            false,
        );
        if height < 20 {
            for (index, card) in self.cards.iter().enumerate() {
                ink!(
                    surface,
                    x,
                    5 + index as i32,
                    body_width,
                    &format!(
                        "{} {} · {}",
                        if self.selected == index { "›" } else { " " },
                        index + 1,
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
                height - 1,
                body_width,
                "1–4 open + Continue · t truth · q quit",
                SOFT,
                body_background,
                false,
            );
            return;
        }
        let columns = if body_width >= 60 { 2 } else { 1 };
        let card_width = (body_width - (columns - 1) * 2) / columns;
        let card_height = 7;
        for (index, card) in self.cards.iter().enumerate() {
            let column = index as i32 % columns;
            let row = index as i32 / columns;
            let card_x = x + column * (card_width + 2);
            let card_y = 6 + row * (card_height + 1);
            if card_y + card_height >= height - 4 {
                continue;
            }
            let selected = index == self.selected;
            let background = if selected { FOCUS_PANEL } else { PANEL };
            let border = if selected { INK } else { SOFT };
            shell_card!(
                surface,
                card_x,
                card_y,
                card_width,
                card_height,
                &format!("{:02} / {}", index + 1, card.lane),
                background,
                border,
            );
            ink!(
                surface,
                card_x + 2,
                card_y + 2,
                card_width - 4,
                card.title,
                INK,
                background,
                true,
            );
            ink!(
                surface,
                card_x + 2,
                card_y + 3,
                card_width - 4,
                card.summary,
                SOFT,
                background,
                false,
            );
            ink!(
                surface,
                card_x + 2,
                card_y + 5,
                card_width - 4,
                card.next,
                INK,
                background,
                false,
            );
            if selected {
                ink!(
                    surface,
                    card_x + card_width - 4,
                    card_y + 1,
                    2,
                    self.motion_mark(),
                    INK,
                    background,
                    true,
                );
            }
        }
        if width >= 118 {
            let system_x = width - 24;
            shell_card!(
                surface,
                system_x,
                3,
                22,
                10,
                "ROCCO / SYSTEM",
                SYSTEM_PANEL,
                INK,
            );
            ink!(
                surface,
                system_x + 2,
                5,
                18,
                "ATTENTION STACK",
                INK,
                SYSTEM_PANEL,
                true,
            );
            ink!(
                surface,
                system_x + 2,
                7,
                18,
                "Moo",
                SOFT,
                SYSTEM_PANEL,
                true,
            );
            ink!(
                surface,
                system_x + 8,
                7,
                11,
                "offline",
                WARN,
                SYSTEM_PANEL,
                false,
            );
            ink!(surface, system_x + 2, 8, 18, "AI", SOFT, SYSTEM_PANEL, true);
            ink!(
                surface,
                system_x + 8,
                8,
                11,
                "offline",
                WARN,
                SYSTEM_PANEL,
                false,
            );
            ink!(
                surface,
                system_x + 2,
                10,
                18,
                "Notes: session only",
                SOFT,
                SYSTEM_PANEL,
                false,
            );
            ink!(
                surface,
                system_x + 2,
                11,
                18,
                "Open truth [t]",
                INK,
                SYSTEM_PANEL,
                true,
            );
        }
        let footer_y = height - 3;
        line(surface, x, footer_y - 1, body_width, SOFT, SHELL);
        ink!(
            surface,
            x,
            footer_y,
            body_width,
            &format!(
                "FOCUS  {}  ·  Enter opens + requests Continue",
                self.cards[self.selected].title
            ),
            ACCENT,
            SHELL,
            true,
        );
        ink!(
            surface,
            x,
            footer_y + 1,
            body_width,
            "← → / Tab select   1–4 open   t truth   q quit",
            SOFT,
            SHELL,
            false,
        );
    }

    fn truth_view(&self, surface: &mut Surface, width: i32, height: i32) {
        let rail = if width >= 92 { 27 } else { 0 };
        if rail > 0 {
            self.sidebar(surface, height);
        }
        let x = rail + 2;
        let body_width = width - x - 2;
        shell_card!(
            surface,
            x,
            3,
            body_width,
            height - 6,
            "TRUTH / DEMO BOUNDARY",
            SYSTEM_PANEL,
            INK,
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
            .flat_map(|line| wrap(line, (width - 4) as usize))
            .collect();
        let scroll = self
            .truth_scroll
            .min(wrapped.len().saturating_sub((height - 5) as usize));
        for (i, line) in wrapped
            .iter()
            .skip(scroll)
            .take((height - 5).max(0) as usize)
            .enumerate()
        {
            ink!(
                surface,
                x + 2,
                i as i32 + 5,
                body_width - 4,
                line,
                INK,
                SYSTEM_PANEL,
                i == 0 || i == 1 || i == 4 || i == 7,
            );
        }
        ink!(
            surface,
            x + 2,
            height - 2,
            body_width - 4,
            "↑ ↓ scroll · Esc / t Back · no state changed",
            ACCENT,
            SYSTEM_PANEL,
            true,
        );
        if width >= 120 && height >= 32 {
            draw_mascot(surface, width - 28, height - 15);
        }
    }

    fn work(&self, surface: &mut Surface, width: i32, height: i32) {
        let card = &self.cards[self.selected];
        let ui = &card.session;
        let rail = if width >= 110 { 27 } else { 0 };
        if rail > 0 {
            self.sidebar(surface, height);
        }
        let x = rail + 2;
        let has_system = width >= 112;
        let right_width = if has_system { 29 } else { 0 };
        let main_width = width - x - right_width - 2;
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
            (height - 12).max(5),
            if context {
                "CONTEXT / DEMO DATA"
            } else {
                "HUMAN + AI"
            },
            SHELL,
            MI,
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
        let space = (height - 15).max(0) as usize;
        let start = if context {
            0
        } else {
            rows.len().saturating_sub(space).saturating_sub(ui.scroll)
        };
        for (i, row) in rows.iter().skip(start).take(space).enumerate() {
            // Authored fixture rows were designed for the old panel. Reflow
            // independently from its width assertions; text still clips visibly.
            let text: String = row.parts.iter().map(|p| p.text.as_str()).collect();
            ink!(
                surface,
                x + 2,
                8 + i as i32,
                main_width - 4,
                &text,
                INK,
                SHELL,
                false,
            );
        }
        if has_system {
            let system_x = width - right_width;
            shell_card!(
                surface,
                system_x,
                6,
                right_width - 2,
                8,
                "ROCCO / SYSTEM",
                SYSTEM_PANEL,
                INK,
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
                system_x + 2,
                8,
                right_width - 6,
                priority,
                WARN,
                SYSTEM_PANEL,
                true,
            );
            ink!(
                surface,
                system_x + 2,
                10,
                right_width - 6,
                detail,
                SOFT,
                SYSTEM_PANEL,
                false,
            );
            ink!(
                surface,
                system_x + 2,
                12,
                right_width - 6,
                "No model. No execution.",
                INK,
                SYSTEM_PANEL,
                true,
            );
            if height >= 26 {
                let context_y = 16;
                let context_h = (height - 22).max(5);
                shell_card!(
                    surface,
                    system_x,
                    context_y,
                    right_width - 2,
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
                    .take((context_h - 2).max(0) as usize)
                    .enumerate()
                {
                    let text: String = row.parts.iter().map(|p| p.text.as_str()).collect();
                    ink!(
                        surface,
                        system_x + 2,
                        context_y + 1 + i as i32,
                        right_width - 6,
                        &text,
                        SOFT,
                        PANEL,
                        false,
                    );
                }
            }
        }
        fill(surface, x, height - 5, main_width, 2, RAIL);
        let draft = if ui.input.is_empty() {
            "Leave a note… (unsent; AI disconnected)"
        } else {
            &ui.input
        };
        ink!(
            surface,
            x + 1,
            height - 4,
            main_width - 2,
            &format!("❯ {draft}"),
            INK,
            RAIL,
            false,
        );
        ink!(
            surface,
            x,
            height - 2,
            main_width,
            "Esc Home  ·  Tab context  ·  /continue  /next  /inspect  /truth",
            SOFT,
            SHELL,
            false,
        );
        ink!(
            surface,
            x,
            height - 1,
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
                self.truth_scroll = (self.truth_scroll + 1).min(24);
            } else if key.code == KeyCode::Up {
                self.truth_scroll = self.truth_scroll.saturating_sub(1);
            }
        } else if !self.entered {
            match key.code {
                KeyCode::Right | KeyCode::Down | KeyCode::Tab => {
                    self.selected = (self.selected + 1) % self.cards.len()
                }
                KeyCode::Left | KeyCode::Up => {
                    self.selected = (self.selected + self.cards.len() - 1) % self.cards.len()
                }
                KeyCode::Enter => self.enter(),
                _ => match ch {
                    '1'..='4' => {
                        self.selected = (ch as u8 - b'1') as usize;
                        self.enter();
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
        if let MouseEvent::Pressed(data) = event {
            let (width, height) = self.size.get();
            if data.y <= 1 && data.x >= width - 22 {
                self.truth = !self.truth;
            } else if self.entered && data.y == 2 && data.x < 8 {
                self.entered = false;
            } else if self.entered && !self.truth && data.y >= height - 4 {
                self.cards[self.selected].session.focus = Focus::Input;
            } else if !self.entered && !self.truth && height < 20 && (5..9).contains(&data.y) {
                self.selected = (data.y - 5) as usize;
                self.enter();
            } else if !self.entered && !self.truth && data.y >= 6 && data.y < 21 {
                let rail = if width >= 92 { 27 } else { 0 };
                let x = rail + 2;
                let right = if width >= 118 { 26 } else { 2 };
                let body_width = width - x - right;
                let count = if body_width >= 60 { 2 } else { 1 };
                let card_width = (body_width - (count - 1) * 2) / count;
                let offset = (data.x - x) / (card_width + 2).max(1);
                let row = (data.y - 6) / 8;
                let selected = row as usize * count as usize + offset as usize;
                if data.x >= x
                    && selected < self.cards.len()
                    && offset >= 0
                    && offset < count
                    && (data.y - 6) % 8 < 7
                {
                    self.selected = selected;
                    self.enter();
                }
            }
        }
        EventProcessStatus::Processed
    }

    fn on_paint(&self, surface: &mut Surface) {
        let width = surface.size().width.min(i32::MAX as u32) as i32;
        let height = surface.size().height.min(i32::MAX as u32) as i32;
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
            width - 26,
            "◉  ROCCO  /  WORKSPACE",
            INK,
            RAIL,
            true,
        );
        ink!(
            surface,
            2,
            1,
            width - 26,
            "You, the system, and an AI — in one place.",
            SOFT,
            RAIL,
            false,
        );
        ink!(
            surface,
            width - 23,
            0,
            21,
            "DEMO DATA  [truth ↗]",
            WARN,
            RAIL,
            false,
        );
        if let Some(error) = &self.setup_error {
            ink!(
                surface,
                2,
                2,
                width - 4,
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
    #![allow(clippy::unwrap_used)]
    use super::*;

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
        (0..height as i32)
            .map(|y| {
                (0..width as i32)
                    .map(|x| surface.char(x, y).unwrap().code)
                    .collect::<String>()
                    + "\n"
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

    fn write_scene_svg(file: &mut std::fs::File, surface: &Surface, width: u32, height: u32) {
        use std::io::Write;
        const CELL_W: i32 = 9;
        const CELL_H: i32 = 18;
        writeln!(file, "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"1120\" height=\"650\" viewBox=\"0 0 1120 650\"><rect width=\"1120\" height=\"650\" fill=\"#0b1020\"/>").unwrap();
        for y in 0..height as i32 {
            let mut start = 0;
            while start < width as i32 {
                let background = surface.char(start, y).unwrap().background;
                let mut end = start + 1;
                while end < width as i32 && surface.char(end, y).unwrap().background == background {
                    end += 1;
                }
                writeln!(
                    file,
                    "<rect x=\"{}\" y=\"{}\" width=\"{}\" height=\"{CELL_H}\" fill=\"{}\"/>",
                    16 + start * CELL_W,
                    8 + y * CELL_H,
                    (end - start) * CELL_W,
                    svg_color(background)
                )
                .unwrap();
                start = end;
            }
            let mut x = 0;
            while x < width as i32 {
                let cell = surface.char(x, y).unwrap();
                let foreground = cell.foreground;
                let background = cell.background;
                let flags = cell.flags;
                let mut text = String::new();
                let mut end = x;
                while end < width as i32 {
                    let next = surface.char(end, y).unwrap();
                    if next.foreground != foreground
                        || next.background != background
                        || next.flags != flags
                    {
                        break;
                    }
                    text.push(next.code);
                    end += 1;
                }
                if !text.trim().is_empty() {
                    let weight = if flags == CharFlags::Bold {
                        " font-weight=\"700\""
                    } else {
                        ""
                    };
                    writeln!(file, "<text x=\"{}\" y=\"{}\" fill=\"{}\" font-family=\"Menlo,monospace\" font-size=\"14\" xml:space=\"preserve\"{weight}>{}</text>", 16 + x * CELL_W, 22 + y * CELL_H, svg_color(foreground), svg_escape(&text)).unwrap();
                }
                x = end;
            }
        }
        writeln!(file, "<text x=\"16\" y=\"625\" fill=\"#a9b5c9\" font-family=\"sans-serif\" font-size=\"12\">Actual AppCUI cell-render preview; not a Terminal screenshot. ANSI16 palette shown faithfully.</text></svg>").unwrap();
    }

    fn press(ui: &mut Workspace, code: KeyCode) {
        ui.on_key_event(Key::new(code, KeyModifier::None), '\0');
    }

    fn command(ui: &mut Workspace, text: &str) {
        ui.cards[ui.selected].session.input = text.into();
        ui.cards[ui.selected].session.command_mode = true;
        press(ui, KeyCode::Enter);
    }

    #[test]
    fn board_is_the_entry_and_truth_survives_small_layouts() {
        let ui = Workspace::default();
        for (width, height) in [(50, 12), (80, 24), (104, 25), (120, 32), (160, 48)] {
            let text = paint(&ui, width, height);
            assert!(text.contains("Your workspace"));
            assert!(text.contains("DEMO DATA"));
            assert!(!text.contains("AI · not connected"));
        }
        assert!(paint(&ui, 120, 32).contains("Needs you"));
        assert!(paint(&ui, 120, 32).contains("Recently done"));
    }

    #[test]
    fn rendered_frames_can_be_reviewed_without_terminal_app_access() {
        let mut ui = Workspace::default();
        let home = paint(&ui, 120, 32);
        ui.enter();
        command(&mut ui, "/next");
        command(&mut ui, "/inspect");
        let work = paint(&ui, 120, 32);
        command(&mut ui, "/truth");
        let truth = paint(&ui, 120, 32);
        assert!(home.contains("Your workspace"));
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
                    .open(path)
                    .unwrap();
                write_scene_svg(&mut file, &scene, 120, 32);
            }
        }
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
                ui.coordinator.board().get(ui.cards[0].run).unwrap().state(),
                RunState::AwaitingAuthority
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
            ui.coordinator.board().get(ui.cards[3].run).unwrap().state(),
            RunState::Completed
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
