//! Keyboard routing and the paint entry point of the native surface.

use super::render::{center, draw_content, draw_input, draw_palette, draw_status, regions};
use super::{
    App, BASE, CharFlags, Character, Color, ERR, EventProcessStatus, Focus, InputApp, Key, KeyCode,
    KeyModifier, MAX_INPUT, MIN_HEIGHT, MIN_WIDTH, Native, Surface, WARN, terminal_dimension,
};

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
            center(surface, 2, width, "✳ Larry DEMO", BASE, CharFlags::Bold);
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
