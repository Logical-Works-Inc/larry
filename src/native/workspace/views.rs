//! The board, truth and work views of the workspace.

use super::{
    BASE, CharAttribute, CharFlags, Character, Color, FOCUS, FOCUS_PANEL, Focus, INK, PANEL, RAIL,
    Row, SHELL, SOFT, SYSTEM_PANEL, Surface, WARN, Work, Workspace, allocation_width, draw_mascot,
    fill, index_coordinate, line, wrap,
};

impl Workspace {
    /// Render the responsive board with explicit disconnected-demo labels.
    pub(super) fn board(&self, surface: &mut Surface, width: i32, height: i32) {
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
                "LARRY / SYSTEM",
                SYSTEM_PANEL,
                SOFT,
            );
            // (column offset, row, width, text, colour, bold): one table, one paint loop.
            let attention: [(i32, i32, i32, &str, Color, bool); 7] = [
                (2, 5, 18, "ATTENTION STACK", INK, true),
                (2, 7, 18, "Moo", SOFT, true),
                (8, 7, 11, "offline", WARN, false),
                (2, 8, 18, "AI", SOFT, true),
                (8, 8, 11, "offline", WARN, false),
                (2, 10, 18, "Notes: session only", SOFT, false),
                (2, 11, 18, "Open truth [t]", INK, true),
            ];
            for (dx, y, span, text, color, bold) in attention {
                ink!(
                    surface,
                    system_x.saturating_add(dx),
                    y,
                    span,
                    text,
                    color,
                    SYSTEM_PANEL,
                    bold,
                );
            }
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
    pub(super) fn truth_view(&self, surface: &mut Surface, width: i32, height: i32) {
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
    pub(super) fn work(&self, surface: &mut Surface, width: i32, height: i32) {
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
                "LARRY / SYSTEM",
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
            // (row, text, colour, bold) of the system card, painted by one loop.
            let status: [(i32, &str, Color, bool); 3] = [
                (8, priority, WARN, true),
                (10, detail, SOFT, false),
                (12, "No model. No execution.", INK, true),
            ];
            for (y, text, color, bold) in status {
                ink!(
                    surface,
                    system_x.saturating_add(2),
                    y,
                    right_width.saturating_sub(6),
                    text,
                    color,
                    SYSTEM_PANEL,
                    bold,
                );
            }
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
