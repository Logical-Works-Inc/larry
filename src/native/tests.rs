use super::render::draw_mascot;
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
fn background_at(ui: &Native, width: u32, height: u32, x: i32, y: i32) -> std::io::Result<Color> {
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
    let expected = [
        // Banner, welcome and the DEMO truth.
        "Larry v",
        "no model, no backend",
        "Safe fixture",
        // Input line with the accent prompt and the right-hand hint.
        "❯",
        "Tab panels",
        // Status hub: rule, MI/MD chips, divider, activity line.
        "MI",
        "MD",
        "│",
        "✢ Ready…",
        "no model called",
        // Dashboard at Forge's threshold: canary, tabs, panel body.
        "CLEAR",
        "▸ Changes",
        "No changes yet",
        "Tip:",
    ];
    for text in expected {
        assert!(shown.contains(text), "the surface is missing {text:?}");
    }
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
