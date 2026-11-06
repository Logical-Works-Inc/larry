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
                // A cell outside the surface reads as blank, as a terminal shows it.
                .map(|x| match surface.char(x, y) {
                    Some(cell) => cell.code,
                    None => ' ',
                })
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
    surface
        .char(x, y)
        .ok_or_else(|| std::io::Error::other(format!("rendered scene has no cell at ({x}, {y})")))
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
            while end < terminal_width && scene_cell(surface, end, y)?.background == background {
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
    if let Some(directory) = std::env::var_os("LARRY_CAPTURE_DIR") {
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
            let path = std::path::PathBuf::from(&directory).join(format!("workspace-{name}.svg"));
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
