//! The question's own view under seeded keys (larry#521). Requirement: no command is approved
//! unread. A command too tall for the screen scrolls, `y` and `a` are refused until every row
//! has been on screen, `n` always works, and every character of the command reaches the screen,
//! a dropped one as its marker. The oracle for "was on screen" is the painted text itself and
//! the oracle for "what the command holds" is written here, never the code's own output.
//! Sources: CWE-451 (UI misrepresentation of critical information); CVE-2021-42574.

use std::collections::BTreeSet;

use super::{header, shown};
use crate::tui::app::{App, Effect, Input, Key};
use crate::tui::worker::{Decision, Event};

/// Seeds the key simulation sweeps.
const SEEDS: u64 = 256;
/// Rows of the tall command.
const ROWS: usize = 40;

/// A frame that asks about `command` on a `width` by `height` screen, painted once.
fn asked(command: &str, width: usize, height: usize) -> App {
    let mut app = App::new(header(), Vec::new(), width, height);
    app.handle(Input::Key(Key::Char('x')));
    app.handle(Input::Key(Key::Enter));
    app.handle(Input::Worker(Event::Ask(command.to_owned())));
    app.draw();
    app
}

/// The command of [`ROWS`] rows, each naming itself.
fn tall() -> String {
    (1..=ROWS)
        .map(|row| format!("echo row-{row:02}"))
        .collect::<Vec<String>>()
        .join("\n")
}

/// Adds to `seen` every row of the tall command the screen shows now.
fn note(app: &App, seen: &mut BTreeSet<usize>) {
    let screen = shown(app);
    seen.extend((1..=ROWS).filter(|row| screen.contains(&format!("echo row-{row:02}"))));
}

/// Whether `effects` run the command.
fn runs(effects: &[Effect]) -> bool {
    effects
        .iter()
        .any(|effect| matches!(*effect, Effect::Decide(Decision::Run | Decision::Always)))
}

/// A splitmix64 step: the one source of every choice in a run.
fn draw(state: &mut u64, bound: usize) -> usize {
    *state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
    let mut mixed = *state;
    mixed = (mixed ^ (mixed >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    mixed = (mixed ^ (mixed >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    mixed ^= mixed >> 31;
    let bound = u64::try_from(bound).into_iter().sum();
    let picked: u64 = mixed.checked_rem(bound).into_iter().sum();
    usize::try_from(picked).into_iter().sum()
}

/// A 40-row command on 80x24: `y` pressed at once runs nothing and says why; under any seeded
/// walk of keys a yes is taken only after every row has been on screen; and paging to the end
/// then `y` runs it.
#[test]
fn ask_shows_every_command_row_before_answer() {
    let mut at_once = asked(&tall(), 80, 24);
    for key in [Key::Char('y'), Key::Char('a')] {
        let effects = at_once.handle(Input::Key(key));
        assert!(
            effects.is_empty(),
            "{key:?} ran an unread command: {effects:?}"
        );
        let screen = shown(&at_once);
        assert!(screen.contains("PgDn first"), "{key:?}: {screen}");
    }
    at_once.handle(Input::Key(Key::Left));
    at_once.handle(Input::Key(Key::Left));
    assert!(
        at_once.handle(Input::Key(Key::Enter)).is_empty(),
        "Enter on Yes ran an unread command"
    );
    assert_eq!(
        at_once.handle(Input::Key(Key::Char('n'))),
        vec![Effect::Decide(Decision::Decline)],
        "n always works"
    );
    // A 16-row screen holds 12 rows of it: both ends seen and the middle not is still unread.
    let mut ends = asked(&tall(), 80, 16);
    ends.handle(Input::Key(Key::End));
    ends.draw();
    assert!(
        ends.handle(Input::Key(Key::Char('y'))).is_empty(),
        "rows 13..28 were never on screen"
    );

    let keys = [
        Key::Up,
        Key::Down,
        Key::PageUp,
        Key::PageDown,
        Key::Home,
        Key::End,
        Key::Char('y'),
        Key::Char('a'),
    ];
    for seed in 0..SEEDS {
        let mut state = seed;
        let mut app = asked(&tall(), 80, 24);
        let mut seen = BTreeSet::new();
        note(&app, &mut seen);
        let mut accepted = false;
        for step in 0..64 {
            if draw(&mut state, 16) == 0 {
                let width = [80, 60, 120].get(draw(&mut state, 3)).copied();
                app.handle(Input::Resize(
                    width.into_iter().sum(),
                    12 + draw(&mut state, 20),
                ));
                app.draw();
                note(&app, &mut seen);
                continue;
            }
            let Some(&key) = keys.get(draw(&mut state, keys.len())) else {
                continue;
            };
            let effects = app.handle(Input::Key(key));
            if runs(&effects) {
                assert_eq!(
                    seen.len(),
                    ROWS,
                    "seed {seed} step {step}: ran unread {seen:?}"
                );
                accepted = true;
                break;
            }
            // A key no frame follows shows nothing, so it can add nothing to what was read.
            if draw(&mut state, 8) != 0 {
                app.draw();
                note(&app, &mut seen);
            }
        }
        if accepted {
            continue;
        }
        app.handle(Input::Resize(80, 24));
        app.draw();
        app.handle(Input::Key(Key::Home));
        app.draw();
        for _ in 0..ROWS {
            app.handle(Input::Key(Key::PageDown));
            app.draw();
        }
        let effects = app.handle(Input::Key(Key::Char('y')));
        assert_eq!(
            effects,
            vec![Effect::Decide(Decision::Run)],
            "seed {seed}: a command read to its end is approved"
        );
    }
}

/// The characters this file puts in a hostile command that no cell may hold.
const DROPPED: [char; 9] = [
    '\u{1b}',
    '\u{7}',
    '\r',
    '\u{9b}',
    '\u{200b}',
    '\u{202e}',
    '\u{2066}',
    '\u{feff}',
    '\u{e0041}',
];

/// What the person must be shown for `command`, written from the rule: every dropped character
/// by its name, a mark with no character before it on its line too, spaces and breaks left out
/// because a row's padding looks like them.
fn wanted(command: &str) -> String {
    let mut out = String::new();
    for line in command.split('\n') {
        for (at, character) in line.chars().enumerate() {
            let bare_mark = at == 0 && character == '\u{301}';
            if bare_mark || DROPPED.contains(&character) {
                out.extend(format!("⟨U+{:04X}⟩", u32::from(character)).chars());
            } else if character != ' ' {
                out.push(character);
            }
        }
    }
    out
}

/// The command rows the screen shows: those between the title and the counted marker, without
/// the margin and the two-cell prompt, and the count the marker names.
fn window(screen: &str) -> (Vec<String>, usize) {
    let mut rows = Vec::new();
    let mut inside = false;
    for line in screen.lines() {
        if line.contains(" cut …") {
            let hidden = line
                .split_whitespace()
                .nth(1)
                .and_then(|count| count.parse::<usize>().ok());
            return (rows, hidden.into_iter().sum());
        }
        if inside {
            rows.push(line.chars().skip(3).collect());
        }
        inside = inside || line.contains("Run this command?");
    }
    (rows, 0)
}

/// A hostile command (control bytes, a bidi override, a format character, wide characters, a
/// bare combining mark, a 10,000-column line and a tail padded out of sight) shows every one of
/// its characters while it is scrolled a row at a time, none of them as an escape, and `y` waits
/// for the last.
#[test]
fn sim_a_hostile_command_shows_every_byte_before_it_runs() {
    let command = [
        "\u{301}printf 'a          b'",
        "    indented\u{1b}[2J\u{7}\r\u{9b}",
        "ad\u{200b}min \u{202e}gnp.exe \u{2066}\u{feff}\u{e0041}",
        "表示幅のテスト😀 e\u{301}",
        &"x".repeat(10_000),
        &format!("true{}; rm -rf /", " ".repeat(2_000)),
    ]
    .join("\n");
    for width in [80, 64, 44] {
        let mut app = asked(&command, width, 24);
        let first = shown(&app);
        assert!(
            first.contains("a          b"),
            "{width}: spaces kept: {first}"
        );
        assert!(
            first.contains("$     indented"),
            "{width}: indent kept: {first}"
        );
        assert!(app.handle(Input::Key(Key::Char('y'))).is_empty());
        let (mut rows, hidden) = window(&shown(&app));
        for step in 0..hidden {
            app.handle(Input::Key(Key::Down));
            let screen = shown(&app);
            assert!(
                !screen.chars().any(|ch| ch != '\n' && DROPPED.contains(&ch)),
                "{width} step {step}: {screen:?}"
            );
            rows.extend(window(&screen).0.pop());
            if step.saturating_add(1) < hidden {
                assert!(
                    app.handle(Input::Key(Key::Char('y'))).is_empty(),
                    "{width} step {step}: ran with rows unread"
                );
            }
        }
        let read: String = rows
            .concat()
            .chars()
            .filter(|&character| character != ' ')
            .collect();
        assert_eq!(read, wanted(&command), "width {width}");
        assert!(shown(&app).contains("rm -rf /"), "{width}: the padded tail");
        assert_eq!(
            app.handle(Input::Key(Key::Char('y'))),
            vec![Effect::Decide(Decision::Run)],
            "width {width}"
        );
    }
}

/// A command that fits asks as before, `y` at once, and PgDn stays the pane's; a screen too
/// small to show the command never approves it and still declines.
#[test]
fn sim_a_short_command_asks_as_before_and_a_blind_screen_never_runs() {
    let mut short = asked("true", 120, 32);
    assert!(short.handle(Input::Key(Key::PageDown)).is_empty());
    short.draw();
    assert_eq!(
        short.handle(Input::Key(Key::Char('y'))),
        vec![Effect::Decide(Decision::Run)]
    );
    for (width, height) in [(80, 1), (2, 24), (0, 0)] {
        let mut blind = asked("true", width, height);
        assert!(
            blind.handle(Input::Key(Key::Char('y'))).is_empty(),
            "{width}x{height} approved a command it never showed"
        );
        assert_eq!(
            blind.handle(Input::Key(Key::Char('n'))),
            vec![Effect::Decide(Decision::Decline)]
        );
    }
}
