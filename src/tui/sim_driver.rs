//! Seeded simulations of the terminal driver's pure parts (larry#239, ADR 0045; larry#576):
//! the key map, the `~` path, the welcome box and `/status` lines, and a start that cannot
//! start. Sources: GNU Readline's emacs bindings (C-a, C-e, C-u, C-k, C-w, C-j, C-h), the
//! kitty keyboard protocol (a release is its own event), CWE-150 and CVE-2021-42574 (escape
//! and bidi text from the environment reaching the person's screen).

use std::collections::BTreeSet;
use std::error::Error;
use std::path::Path;

use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use lgwks_std::seeded::Seeded;

use crate::executor::{ExecError, Settings};
use crate::sim_error::SimError;
use crate::tui::app::{App, Effect, Input, Key};
use crate::tui::cells::{Entry, Header};
use crate::tui::style::{Style, text_width};
use crate::tui::{STATUS_REASON, header, key, start, tilde};

/// What every test returns.
type TestResult = Result<(), Box<dyn Error>>;

/// Seeds the key simulation runs.
const SEEDS: u64 = 4096;

/// Every code the simulation draws: the ones the frame binds and ones it must ignore.
const CODES: [KeyCode; 26] = [
    KeyCode::Enter,
    KeyCode::Backspace,
    KeyCode::Delete,
    KeyCode::Left,
    KeyCode::Right,
    KeyCode::Up,
    KeyCode::Down,
    KeyCode::Home,
    KeyCode::End,
    KeyCode::PageUp,
    KeyCode::PageDown,
    KeyCode::Tab,
    KeyCode::Esc,
    KeyCode::BackTab,
    KeyCode::Insert,
    KeyCode::F(1),
    KeyCode::F(12),
    KeyCode::Null,
    KeyCode::CapsLock,
    KeyCode::ScrollLock,
    KeyCode::NumLock,
    KeyCode::PrintScreen,
    KeyCode::Pause,
    KeyCode::Menu,
    KeyCode::KeypadBegin,
    KeyCode::Char('\0'),
];

/// Characters the simulation types: every bound letter, its neighbours and hostile text.
const CHARS: [char; 17] = [
    'a', 'c', 'd', 'e', 'h', 'j', 'k', 'u', 'w', 'C', 'x', 'y', 'n', ' ', '\u{1b}', '\u{202e}',
    '界',
];

/// The readline control bindings, written here from the manual, not from the driver.
const CONTROL: [(char, Key); 9] = [
    ('a', Key::Home),
    ('c', Key::Interrupt),
    ('d', Key::EndOfInput),
    ('e', Key::End),
    ('h', Key::Backspace),
    ('j', Key::NewLine),
    ('k', Key::KillEnd),
    ('u', Key::KillStart),
    ('w', Key::KillWord),
];

/// The unmodified keys the frame binds.
const PLAIN: [(KeyCode, Key); 12] = [
    (KeyCode::Backspace, Key::Backspace),
    (KeyCode::Delete, Key::Delete),
    (KeyCode::Left, Key::Left),
    (KeyCode::Right, Key::Right),
    (KeyCode::Up, Key::Up),
    (KeyCode::Down, Key::Down),
    (KeyCode::Home, Key::Home),
    (KeyCode::End, Key::End),
    (KeyCode::PageUp, Key::PageUp),
    (KeyCode::PageDown, Key::PageDown),
    (KeyCode::Tab, Key::Tab),
    (KeyCode::Esc, Key::Esc),
];

/// The reference key map over the two tables.
fn reference(code: KeyCode, modifiers: KeyModifiers, kind: KeyEventKind) -> Option<Key> {
    if kind == KeyEventKind::Release {
        return None;
    }
    if let KeyCode::Char(typed) = code {
        if modifiers.contains(KeyModifiers::CONTROL) {
            return CONTROL
                .iter()
                .find(|&&(letter, _)| letter == typed)
                .map(|&(_, bound)| bound);
        }
        return Some(Key::Char(typed));
    }
    if code == KeyCode::Enter {
        let soft = modifiers.intersects(KeyModifiers::ALT | KeyModifiers::SHIFT);
        return Some(if soft { Key::NewLine } else { Key::Enter });
    }
    PLAIN
        .iter()
        .find(|&&(known, _)| known == code)
        .map(|&(_, bound)| bound)
}

/// fat-finger and liar-upstream: for 4,096 seeds a key event of any code, any modifier set and
/// any kind maps to what the reference says. Catches a release that acts, a Ctrl+letter that
/// types the letter into a command, an unbound Ctrl chord or function key that becomes input,
/// and Shift+Enter that submits.
#[test]
fn sim_driver_every_terminal_key_maps_to_one_frame_key() -> TestResult {
    let flags = [
        KeyModifiers::SHIFT,
        KeyModifiers::CONTROL,
        KeyModifiers::ALT,
        KeyModifiers::SUPER,
        KeyModifiers::META,
    ];
    let kinds = [
        KeyEventKind::Press,
        KeyEventKind::Repeat,
        KeyEventKind::Release,
    ];
    let mut bound = BTreeSet::new();
    for seed in 0..SEEDS {
        let mut rng = Seeded::from_seed(seed);
        let code = if rng.below(2)? == 0 {
            KeyCode::Char(*CHARS.get(rng.index(CHARS.len())?).ok_or("chars")?)
        } else {
            *CODES.get(rng.index(CODES.len())?).ok_or("codes")?
        };
        let mut modifiers = KeyModifiers::NONE;
        for flag in flags {
            if rng.below(3)? == 0 {
                modifiers |= flag;
            }
        }
        let kind = *kinds.get(rng.index(kinds.len())?).ok_or("kinds")?;
        let want = reference(code, modifiers, kind);
        assert_eq!(
            key(KeyEvent::new_with_kind(code, modifiers, kind)),
            want,
            "seed {seed}: {code:?} {modifiers:?} {kind:?}"
        );
        if let Some(mapped) = want {
            bound.insert(format!("{mapped:?}"));
        }
    }
    for &(letter, wanted) in &CONTROL {
        let event = KeyEvent::new(KeyCode::Char(letter), KeyModifiers::CONTROL);
        assert_eq!(key(event), Some(wanted), "Ctrl+{letter}");
        assert!(
            bound.contains(&format!("{wanted:?}")),
            "the seeds reached {wanted:?}"
        );
    }
    Ok(())
}

/// hostile-path (CWE-706): only a whole leading home component is written `~`. Catches
/// a sibling such as `alice` shown as `~ice` for the home `al`, and an empty `HOME` that would
/// claim every path is under home.
#[test]
fn sim_driver_tilde_shortens_only_whole_home_components() {
    let home = Some("/u/al");
    let cases = [
        ("/u/al", home, "~"),
        ("/u/al/", home, "~"),
        ("/u/al/work/larry", home, "~/work/larry"),
        ("/u/alice/work", home, "/u/alice/work"),
        ("/srv/u/al", home, "/srv/u/al"),
        ("/u", home, "/u"),
        ("/u/al/work", None, "/u/al/work"),
        ("/u/al/work", Some(""), "/u/al/work"),
        ("/u/al/work", Some("u/al"), "/u/al/work"),
    ];
    for (path, home, want) in cases {
        assert_eq!(tilde(Path::new(path), home), want, "{path} under {home:?}");
    }
}

/// Settings over `state` plus the named variables.
fn settings(state: &Path, extra: &[(&str, &str)]) -> Result<Settings, ExecError> {
    let state = state.to_string_lossy().into_owned();
    let extra: Vec<(String, String)> = extra
        .iter()
        .map(|&(name, value)| (name.to_owned(), value.to_owned()))
        .collect();
    Settings::from_env(move |name| match name {
        "LARRY_STATE_DIR" => Some(state.clone()),
        "PATH" => Some(String::from("bin")),
        other => extra
            .iter()
            .find(|pair| pair.0 == other)
            .map(|pair| pair.1.clone()),
    })
}

/// The `/status` line that starts with `lead`.
fn line<'lines>(status: &'lines [String], lead: &'static str) -> Result<&'lines str, SimError> {
    status
        .iter()
        .map(String::as_str)
        .find(|line| line.starts_with(lead))
        .ok_or(SimError::Missing(lead))
}

/// first-run and operator: with no route the welcome box and `/status` say `none` and why;
/// with the environment route they name its model and never its key; the directory is shown
/// under `~`. Catches a first-time user shown a model that would not be called, or no reason
/// why none will be.
#[test]
fn sim_driver_header_names_the_model_or_why_there_is_none() -> TestResult {
    let root = crate::executor::sim_tree::scratch("tui-driver-model")?;
    let home = root.to_string_lossy().into_owned();
    let cwd = root.join("work");

    let (welcome, status) = header(&settings(&root, &[]), &cwd, Some(&home));
    let why = match welcome.model {
        Err(ref why) => why.clone(),
        Ok(ref name) => return Err(format!("no route was configured, got {name}").into()),
    };
    assert!(why.starts_with("none (") && why.contains("route"), "{why}");
    assert_eq!(line(&status, "model:")?, format!("model: {why}"));
    assert_eq!(welcome.directory, "~/work");
    assert_eq!(line(&status, "directory:")?, "directory: ~/work");
    assert_eq!(welcome.version, env!("CARGO_PKG_VERSION"));

    let routed = settings(
        &root,
        &[
            ("LARRY_MODEL_URL", "http://127.0.0.1:1/v1"),
            ("LARRY_MODEL", "sim-model-7"),
            ("LARRY_MODEL_KEY", "never-shown-secret"),
        ],
    );
    let (welcome, status) = header(&routed, &cwd, None);
    assert_eq!(welcome.model, Ok(String::from("sim-model-7")));
    assert_eq!(line(&status, "model:")?, "model: sim-model-7");
    assert_eq!(welcome.directory, cwd.display().to_string());
    assert!(
        !format!("{welcome:?}{status:?}").contains("never-shown-secret"),
        "the key never reaches the screen"
    );
    Ok(())
}

/// Every painted row of a fresh frame showing `welcome`.
fn painted(welcome: Header, status: Vec<String>) -> Vec<String> {
    let app = App::new(welcome, status, 80, 24);
    let frame = app.draw();
    (0..24).map(|row| frame.buffer.row_text(row)).collect()
}

/// weird-bytes (CWE-150, CVE-2021-42574): an environment whose refused value holds an escape
/// sequence, a bidi override and 4,000 columns reaches `/status` as one clean line of at most
/// [`STATUS_REASON`] columns for the reason, the model reads `none`, and nothing painted holds
/// a control or bidi byte. Catches a hostile variable that retitles or rewrites the terminal.
#[test]
fn sim_driver_header_sanitizes_and_bounds_a_hostile_refusal() -> TestResult {
    let root = crate::executor::sim_tree::scratch("tui-driver-hostile")?;
    let value = format!("\u{1b}]0;owned\u{7}\u{202e}{}", "x".repeat(4000));
    let refused = settings(&root, &[("LARRY_POLICY", &value)]);
    assert!(
        matches!(refused, Err(ExecError::BadPolicy { .. })),
        "the hostile policy name is the refusal"
    );
    let (welcome, status) = header(&refused, &root, None);
    assert_eq!(welcome.model, Err(String::from("none")));
    assert_eq!(line(&status, "model:")?, "model: none");
    let commands = line(&status, "commands:")?;
    let reason = commands
        .strip_prefix("commands: off (")
        .and_then(|rest| rest.strip_suffix(')'))
        .ok_or_else(|| format!("not an off line: {commands}"))?;
    assert!(
        reason.starts_with("exec LARRY_POLICY is not a scope name:"),
        "{reason}"
    );
    assert!(
        text_width(reason) <= STATUS_REASON,
        "{} columns",
        text_width(reason)
    );
    assert!(
        !commands
            .chars()
            .any(|shown| shown.is_control() || shown == '\u{202e}'),
        "{commands:?}"
    );
    for row in painted(welcome, status) {
        assert!(
            !row.chars()
                .any(|shown| shown.is_control() || shown == '\u{202e}'),
            "{row:?}"
        );
    }
    Ok(())
}

/// broken-machine: a task sent while the environment is refused starts no session, and the
/// frame returns to idle with the typed refusal as its notice. Catches a frame left spinning
/// with no session behind it, or a start that runs on defaults after a refused setting.
#[test]
fn sim_driver_start_with_refused_settings_runs_nothing_and_says_why() -> TestResult {
    let root = crate::executor::sim_tree::scratch("tui-driver-start")?;
    let refused = settings(&root, &[("LARRY_EXEC_TIMEOUT_MS", "0")]);
    let (welcome, status) = header(&refused, &root, None);
    let mut app = App::new(welcome, status, 80, 24);
    for typed in "build it".chars() {
        app.handle(Input::Key(Key::Char(typed)));
    }
    let effects = app.handle(Input::Key(Key::Enter));
    assert_eq!(effects, vec![Effect::Start(String::from("build it"))]);
    assert!(app.busy(), "the frame waits on the session it asked for");
    let worker = start(&mut app, &refused, String::from("build it"), &root);
    assert!(worker.is_none(), "no session starts");
    assert!(!app.busy(), "the frame is idle again");
    assert_eq!(
        app.entries().back(),
        Some(&Entry::Notice(
            Style::BAD,
            String::from("exec LARRY_EXEC_TIMEOUT_MS is not a timeout in 1..=86400000: 0")
        ))
    );
    Ok(())
}
