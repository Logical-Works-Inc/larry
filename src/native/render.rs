//! Painting: the mascot, rows, frames and every region of the Forge-shaped surface.

use super::{
    ACCENT, ACTIVITY_SUFFIX, BASE, CHIP, CharAttribute, CharFlags, Character, Color,
    DASHBOARD_MIN_WIDTH, Fit, Focus, HINT, MI, MUTED, Native, PANEL_MIN_INNER, PROMPT, Part, Row,
    Surface, WARN, Work, allocation_width, chip, dashboard_width, hub_mi_width, index_coordinate,
    part, text_width,
};

// ── painting primitives ──────────────────────────────────────────────────

/// Blocky Larry — the 24x24 pixel raccoon, hand-authored from the canonical
/// larry-flat.webp (Director request 2026-09-08: flatter, symmetric Larry) by
/// tools/larry_pixel.py, which owns the blink frame and the Codex pet export.
/// Identity features preserved per Director history: paired dark-interior
/// ears, bandit mask, LARGE brown eyes with cream catchlights, dark nose,
/// open smile with tongue. Voice: warm, calm, concrete — never decoration.
pub(super) const MASCOT: [&str; 24] = [
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
pub(super) fn mascot_color(ch: char) -> Option<Color> {
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
pub(super) fn draw_mascot(surface: &mut Surface, x: i32, y: i32) {
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
pub(super) fn write(
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
pub(super) fn write_row(surface: &mut Surface, x: i32, y: i32, width: i32, row: &Row) {
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
pub(super) fn rule(surface: &mut Surface, x: i32, y: i32, width: i32) {
    let line: String = "─".repeat(allocation_width(width));
    write(surface, x, y, width, &line, MUTED, CharFlags::None);
}

/// Square panel frame with an optional title on the top edge, matching
/// Forge's `Borders::ALL`. Content rows go inside at `x + 2`, starting at
/// `y + 1`. Returns the inner width.
pub(super) fn frame(
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
pub(super) fn center(
    surface: &mut Surface,
    y: i32,
    width: i32,
    text: &str,
    color: Color,
    flags: CharFlags,
) {
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
pub(super) struct Regions {
    /// First transcript row.
    pub(super) content_top: i32,
    /// Exclusive transcript bottom row.
    pub(super) content_bottom: i32,
    /// Input row.
    pub(super) input_y: i32,
    /// Separator immediately above the status hub.
    pub(super) hub_rule_y: i32,
    /// First status-hub row.
    pub(super) hub_top_y: i32,
    /// Final status-hub row.
    pub(super) hub_bottom_y: i32,
}

/// Computes the terminal rows assigned to transcript, input, and status hub.
pub(super) fn regions(height: i32) -> Regions {
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
pub(super) fn draw_transcript(surface: &mut Surface, width: i32, area: (i32, i32), ui: &Native) {
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
pub(super) fn draw_panel(
    surface: &mut Surface,
    width: i32,
    area: (i32, i32),
    title: &str,
    rows: &[Row],
) {
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
/// reason in the same colour. Forge's one-glance strip, Larry's real state.
pub(super) fn draw_canary(surface: &mut Surface, x: i32, y: i32, width: i32, ui: &Native) {
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
pub(super) const DASH_TITLE: &str = " fixture state";

/// Forge's `·`-divided tab bar; the active tab keeps the `▸` marker so the
/// selection is not carried by colour alone.
pub(super) fn draw_tabs(surface: &mut Surface, x: i32, y: i32, width: i32, ui: &Native) {
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
pub(super) fn draw_dashboard(surface: &mut Surface, width: i32, area: (i32, i32), ui: &Native) {
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
        inner_w >= index_coordinate(PANEL_MIN_INNER) || width < DASHBOARD_MIN_WIDTH,
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
pub(super) fn draw_content(surface: &mut Surface, width: i32, area: (i32, i32), ui: &Native) {
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
pub(super) fn draw_input(surface: &mut Surface, width: i32, y: i32, ui: &Native) {
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
/// MD (the fixture's machine side) split by a vertical divider. Larry already
/// names these halves MI and MD (D01), so the hub is a direct fit.
pub(super) fn draw_status(surface: &mut Surface, width: i32, regions: &Regions, ui: &Native) {
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
                    super::super::EVENTS.len(),
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
pub(super) fn write_parts(
    surface: &mut Surface,
    x: i32,
    y: i32,
    width: i32,
    parts: &[Part],
) -> i32 {
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
pub(super) fn draw_palette(surface: &mut Surface, width: i32, input_y: i32, ui: &Native) {
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
