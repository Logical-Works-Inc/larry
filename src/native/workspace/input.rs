//! Keyboard, pointer and paint routing for the workspace.

use super::{
    App, CharAttribute, CharFlags, Character, ERR, EventProcessStatus, Focus, INK, InputApp, Key,
    KeyCode, KeyModifier, MouseEvent, RAIL, RAIL_ITEM_HEIGHT, RAIL_ITEM_WIDTH, RAIL_ITEM_X,
    RAIL_ITEM_Y, SHELL, SOFT, Surface, WARN, Workspace, allocation_width, fill, line, rail_item_y,
    terminal_dimension,
};

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
                "Larry · resize to 50x12 · Ctrl-C exits",
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
            "◉  LARRY  /  WORKSPACE",
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
