//! Seeded simulations of the terminal frame (larry#239, ADR 0045). One seed
//! drives every size, key, paste and session event; no clock, no terminal.
//! Sources: ECMA-48 SGR and CUP; the terminal-wg synchronized-output spec
//! (DEC 2026); CVE-2021-42574 (Trojan Source bidi) and CVE-2003-0063 (xterm
//! title-report escape injection) for the hostile-output cases.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash as _, Hasher as _};
use std::time::Duration;

use crate::executor::Isolation;
use crate::sim_lanes::sweep_on;

/// A paint is CPU-bound, so every sweep here runs on one lane (see [`sweep_on`]); the dev
/// profile's optimized code is what makes each paint cheaper (ADR 0049).
const PAINT_LANES: u64 = 1;

use super::app::{App, Input, Key, MAX_ENTRIES};
use super::buffer::{Buffer, Frame, render};
use super::cells::Header;
use super::layout::{CHAT_MAX, CHAT_MIN, SPLIT_MIN, split};
use super::style::Style;
use super::worker::{Ended, Event};

/// Seeds each simulation runs.
const SEEDS: u64 = 256;

/// A xorshift64* stream: the one source of every choice in a run.
pub(super) struct Seeded(u64);

impl Seeded {
    /// A stream for `seed`; zero is moved off the fixed point.
    pub(super) fn new(seed: u64) -> Self {
        Self(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1)
    }

    /// The next value.
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    /// A value in `0..bound`, or 0 for an empty bound.
    fn below(&mut self, bound: u64) -> u64 {
        self.next().checked_rem(bound).into_iter().sum()
    }

    /// An index in `0..len`.
    pub(super) fn pick(&mut self, len: usize) -> usize {
        let bound = u64::try_from(len).into_iter().sum();
        usize::try_from(self.below(bound)).into_iter().sum()
    }

    /// The item of `items` this stream picks, or the empty text for an empty list.
    pub(super) fn word<'items>(&mut self, items: &[&'items str]) -> &'items str {
        match items.get(self.pick(items.len())) {
            Some(&item) => item,
            None => "",
        }
    }
}

/// Text a hostile model or command might print: escapes, controls, bidi, wide and
/// every invisible format character that could hide a difference between the command
/// shown and the command that runs (larry#337).
pub(super) const HOSTILE: [&str; 16] = [
    "\x1b]0;owned\x07",
    "\x1b[2J\x1b[H",
    "\x1b[31mred\x1b[0m",
    "\u{202e}gnp.exe",
    "\u{2066}if admin\u{2069}",
    "表示幅のテスト😀",
    "e\u{301}\u{301}\u{301}",
    "\r\x08\x08\x00tab\there",
    "\u{9b}31m4:3R",
    "\x1b]8;;https://evil.example\x1b\\click\x1b]8;;\x1b\\",
    "\x1b]52;c;ZWNobyBoaQ==\x07",
    "\x1bP1;2|payload\x1b\\",
    "ad\u{200b}min\u{200c}\u{200d}",
    "hy\u{00ad}phen\u{feff}",
    "a\u{e0041}\u{e0042}\u{e007f}b",
    "line\u{2028}para\u{2029}graph",
];

/// A header for a simulated run.
fn header() -> Header {
    Header {
        version: String::from("0.0.0"),
        model: Ok(String::from("sim-model")),
        directory: String::from("~/sim"),
        blocked: None,
    }
}

/// One seeded input.
fn input(rng: &mut Seeded) -> Input {
    let text = |rng: &mut Seeded| rng.word(&HOSTILE).to_owned();
    match rng.below(14) {
        0 => Input::Resize(rng.pick(260), rng.pick(90)),
        1 => Input::Paste(text(rng)),
        2 => Input::Worker(Event::Ask(text(rng))),
        3 => Input::Worker(Event::Started(text(rng))),
        4 => Input::Worker(Event::Finished {
            observation: format!("exit {}\n{}", rng.below(3), text(rng)),
            passed: rng.below(2) == 0,
        }),
        5 => Input::Worker(Event::Said(text(rng))),
        6 => Input::Worker(Event::Ended(Ended::Done)),
        7 => Input::Worker(Event::Ended(Ended::Failed(text(rng)))),
        8 => Input::Tick(Duration::from_millis(rng.below(100_000))),
        _ => Input::Key(super::sim_parts::key_at(
            rng.pick(super::sim_parts::KEY_COUNT),
        )),
    }
}

/// Runs `steps` seeded inputs through a fresh app; returns the app and every
/// byte the driver would have written.
fn drive(seed: u64, steps: usize) -> (App, Vec<u8>) {
    let mut rng = Seeded::new(seed);
    let mut app = App::new(header(), vec![String::from("model: sim")], 120, 34);
    let mut shown: Option<Frame> = None;
    let mut wire = Vec::new();
    for _ in 0..steps {
        app.handle(input(&mut rng));
        let frame = app.draw();
        render(shown.as_ref(), &frame, &mut wire);
        shown = Some(frame);
    }
    (app, wire)
}

/// Every SGR parameter list in `wire`, in order, with the text after it.
fn sgr_runs(wire: &[u8]) -> Vec<(Vec<u32>, String)> {
    let text = String::from_utf8_lossy(wire);
    let mut runs = Vec::new();
    for piece in text.split("\x1b[").skip(1) {
        let Some(end) = piece.find(|character: char| character.is_ascii_alphabetic()) else {
            continue;
        };
        let (params, rest) = piece.split_at(end);
        if !rest.starts_with('m') {
            continue;
        }
        let codes = params
            .split(';')
            .filter_map(|code| code.parse().ok())
            .collect();
        runs.push((codes, rest.get(1..).into_iter().collect()));
    }
    runs
}

#[test]
fn layout_chat_column_keeps_phone_ratio_and_reserves_artifact_pane() {
    for width in 0..=400 {
        for height in [0, 1, 10, 24, 34, 48, 69, 70, 120, 500] {
            let regions = split(width, height);
            assert_eq!(regions.chat.x, 0);
            assert_eq!(regions.chat.height, height);
            match regions.artifact {
                None => {
                    assert!(width < SPLIT_MIN, "{width}x{height} lost its pane");
                    assert_eq!(regions.chat.width, width);
                }
                Some(pane) => {
                    assert!(width >= SPLIT_MIN);
                    assert!((CHAT_MIN..=CHAT_MAX).contains(&regions.chat.width));
                    // One rule cell between the chat and the pane; the pane takes the rest.
                    assert_eq!(pane.x, regions.chat.width.saturating_add(1));
                    assert_eq!(pane.right(), width);
                    assert!(pane.width >= SPLIT_MIN.saturating_sub(CHAT_MAX).saturating_sub(1));
                }
            }
        }
    }
    // A phone is 9:19.5 and a cell about 1:2, so 52 rows want 48 columns.
    assert_eq!(split(160, 52).chat.width, 48);
}

#[test]
fn sim_tui_any_size_any_input_paints_inside_bounds() -> Result<(), Box<dyn std::error::Error>> {
    sweep_on(PAINT_LANES, SEEDS, |_lane, seed| {
        let (app, _) = drive(seed, 96);
        let frame = app.draw();
        let grid: &Buffer = &frame.buffer;
        assert!(grid.width() <= super::buffer::MAX_COLUMNS);
        assert!(grid.height() <= super::buffer::MAX_ROWS);
        if let Some((x, y)) = frame.cursor {
            assert!(
                x < grid.width() && y < grid.height(),
                "seed {seed}: cursor off grid"
            );
        }
    })?;
    Ok(())
}

#[test]
fn sim_tui_hostile_output_cannot_write_escapes() -> Result<(), Box<dyn std::error::Error>> {
    sweep_on(PAINT_LANES, SEEDS, |_lane, seed| {
        let (_, wire) = drive(seed, 96);
        let text = String::from_utf8_lossy(&wire);
        for piece in text.split('\x1b').skip(1) {
            // Only the frame's own sequences: CSI with digits, `;` and `?` then a final.
            let ok = piece.starts_with('[')
                && piece
                    .chars()
                    .skip(1)
                    .find(|character| {
                        !(character.is_ascii_digit() || matches!(character, ';' | '?'))
                    })
                    .is_some_and(|last| matches!(last, 'H' | 'J' | 'm' | 'h' | 'l'));
            assert!(ok, "seed {seed}: foreign escape {:?}", piece.get(..12));
        }
        assert!(
            !text.contains("\x1b]"),
            "seed {seed}: an OSC reached the wire"
        );
        for banned in [
            '\u{7}',
            '\u{8}',
            '\0',
            '\r',
            '\u{9b}',
            '\u{00ad}',
            '\u{200b}',
            '\u{200c}',
            '\u{200d}',
            '\u{200e}',
            '\u{200f}',
            '\u{2028}',
            '\u{2029}',
            '\u{202a}',
            '\u{202b}',
            '\u{202c}',
            '\u{202d}',
            '\u{202e}',
            '\u{2060}',
            '\u{2061}',
            '\u{2062}',
            '\u{2063}',
            '\u{2064}',
            '\u{2066}',
            '\u{2067}',
            '\u{2068}',
            '\u{2069}',
            '\u{feff}',
            '\u{e0041}',
            '\u{e007f}',
        ] {
            assert!(
                !text.contains(banned),
                "seed {seed}: {banned:?} reached the wire"
            );
        }
    })?;
    Ok(())
}

#[test]
fn sim_tui_bold_cells_keep_colour_on_the_wire() -> Result<(), Box<dyn std::error::Error>> {
    let saw = sweep_on(PAINT_LANES, SEEDS, |_lane, seed| {
        let mut saw_bold_colour = false;
        let (_, wire) = drive(seed, 64);
        for (codes, _) in sgr_runs(&wire) {
            // Every style run starts from reset: nothing of the last run survives.
            assert_eq!(
                codes.first(),
                Some(&0),
                "seed {seed}: SGR {codes:?} without reset"
            );
            // A reset never follows the bold or the colour inside one run.
            assert!(
                !codes.iter().skip(1).any(|code| *code == 0),
                "seed {seed}: {codes:?}"
            );
            if codes.contains(&1) && codes.iter().any(|code| (30..=37).contains(code)) {
                saw_bold_colour = true;
            }
        }
        saw_bold_colour
    })?;
    let mut wire = Vec::new();
    Style::ACCENT.bold().sgr(&mut wire);
    assert_eq!(wire, b"\x1b[0;1;36m");
    assert!(
        saw.into_iter().any(|bold| bold),
        "no seed painted a bold coloured cell"
    );
    Ok(())
}

#[test]
fn sim_tui_same_seed_same_frame_hash() -> Result<(), Box<dyn std::error::Error>> {
    let hash = |seed: u64| {
        let (_, wire) = drive(seed, 128);
        let mut hasher = DefaultHasher::new();
        wire.hash(&mut hasher);
        hasher.finish()
    };
    let hashes = sweep_on(PAINT_LANES, 64, |_lane, seed| {
        let first = hash(seed);
        assert_eq!(first, hash(seed), "seed {seed} replayed differently");
        first
    })?;
    let distinct: std::collections::BTreeSet<u64> = hashes.into_iter().collect();
    assert!(
        distinct.len() > 32,
        "seeds collapse to {} traces",
        distinct.len()
    );
    Ok(())
}

#[test]
fn sim_tui_unchanged_frame_writes_nothing() -> Result<(), Box<dyn std::error::Error>> {
    sweep_on(PAINT_LANES, SEEDS, |_lane, seed| {
        let (app, _) = drive(seed, 48);
        let frame = app.draw();
        let mut wire = Vec::new();
        render(Some(&frame), &frame, &mut wire);
        assert!(
            wire.is_empty(),
            "seed {seed}: an idle repaint wrote {} bytes",
            wire.len()
        );
        render(None, &frame, &mut wire);
        assert!(wire.starts_with(b"\x1b[?2026h"));
        assert!(wire.ends_with(b"\x1b[?2026l"));
    })?;
    Ok(())
}

#[test]
fn sim_tui_transcript_and_events_stay_bounded_past_cap() {
    let mut app = App::new(header(), Vec::new(), 120, 34);
    let past = MAX_ENTRIES.saturating_mul(3);
    for step in 0..past {
        app.handle(Input::Worker(Event::Finished {
            observation: format!("exit 0\nline {step}"),
            passed: true,
        }));
    }
    assert_eq!(app.entries().len(), MAX_ENTRIES);
    // The newest survives; the header and the oldest went first.
    let frame = app.draw();
    let rows: String = (0..frame.buffer.height())
        .map(|row| frame.buffer.row_text(row))
        .collect();
    assert!(rows.contains(&format!("line {}", past.saturating_sub(1))));
    assert_eq!(super::worker::EVENTS, 64);
}

#[test]
fn sim_tui_esc_answers_a_question_with_decline() {
    for seed in 0..SEEDS {
        let mut rng = Seeded::new(seed);
        let mut app = App::new(header(), Vec::new(), 40 + rng.pick(200), 10 + rng.pick(60));
        app.handle(Input::Key(Key::Char('x')));
        let started = app.handle(Input::Key(Key::Enter));
        assert_eq!(started, vec![super::app::Effect::Start(String::from("x"))]);
        app.handle(Input::Worker(Event::Ask(String::from("rm -rf /"))));
        let answer = app.handle(Input::Key(Key::Esc));
        assert_eq!(
            answer,
            vec![super::app::Effect::Decide(super::worker::Decision::Decline)],
            "seed {seed}"
        );
    }
}

/// The bound one worker event wait may take before the session is called broken (larry#268):
/// the event ends the wait, and a session that sends nothing fails by name instead of ending
/// the loop early and reporting the wrong last event. This file reads no clock; the wait is
/// the channel's own, and the bound only turns a session that never answers into a failure.
const WORKER_PATIENCE: Duration = Duration::from_secs(120);

/// Every event the worker sends, up to and including its `Ended`; `None` when the session sent
/// nothing inside [`WORKER_PATIENCE`], so a caller fails the run by name instead of ending the
/// loop early.
fn every_event(worker: &super::worker::Worker) -> Option<Vec<Event>> {
    let mut seen = Vec::new();
    loop {
        let event = worker.wait(WORKER_PATIENCE)?;
        let last = matches!(event, Event::Ended(_));
        seen.push(event);
        if last {
            return Some(seen);
        }
    }
}

/// Settings over `root` with a model route to the discard port, which nothing serves: a
/// session that reaches the model fails loudly, and these sims never let it.
fn unreachable_model(
    root: &std::path::Path,
) -> Result<crate::executor::Settings, crate::executor::ExecError> {
    let state = root.to_string_lossy().into_owned();
    crate::executor::Settings::from_env(move |name| match name {
        "LARRY_STATE_DIR" => Some(state.clone()),
        "LARRY_MODEL_URL" => Some(String::from("http://127.0.0.1:9/v1")),
        "LARRY_MODEL" => Some(String::from("sim-model")),
        "LARRY_MODEL_KEY" => Some(String::from("sim-key")),
        _ => None,
    })
}

/// The real `run_session` behind the worker, with no in-crate stand-in: under the default
/// policy (no `model.command`, `executor::gate`) the gate refuses before any model call, the
/// refusal reaches the frame as events ending in `Ended::Refused`, and dropping a worker
/// mid-flight joins its thread.
#[test]
fn sim_tui_worker_drives_run_session_through_observer() -> Result<(), Box<dyn std::error::Error>> {
    // The tree a dead run with this process id left at the old `larry-tui-worker-<pid>` name is
    // here, and the fixture starts empty anyway (larry#267).
    let stale = crate::executor::sim_tree::leftover("larry-tui-worker", "hello.txt")?;
    assert!(
        stale.display().to_string().contains("larry-tui-worker-"),
        "the leftover sits at the old name: {}",
        stale.display()
    );
    assert!(
        stale.join("hello.txt").exists(),
        "the old leftover is present"
    );
    let root = crate::executor::sim_tree::scratch("tui-worker")?;
    let settings = unreachable_model(&root)?;
    let worker = super::worker::Worker::spawn(
        String::from("write hello.txt"),
        root.to_path_buf(),
        settings.clone(),
        Isolation::Host,
        false,
    )?;
    let seen = every_event(&worker).ok_or("the session sent no event inside its bound")?;
    assert_eq!(seen.last(), Some(&Event::Ended(Ended::Refused)), "{seen:?}");
    // The sealed refusal reaches the frame as a receipt: not accepted, nothing ran.
    let Some(before_end) = seen.iter().rev().nth(1) else {
        return Err("no event before the end".into());
    };
    let Event::Receipt(ref receipt) = *before_end else {
        return Err(format!("expected a receipt before the end, got {before_end:?}").into());
    };
    assert!(!receipt.accepted, "{receipt:?}");
    assert_eq!(
        (
            receipt.commands,
            receipt.handoff.as_str(),
            receipt.digest.len()
        ),
        (0, "none", 12)
    );
    assert!(
        !seen.iter().any(|event| matches!(*event, Event::Ask(_))),
        "{seen:?}"
    );
    assert!(
        !root.join("hello.txt").exists(),
        "a refused session runs nothing"
    );
    // A worker dropped before its session reports is cancelled and joined, not leaked.
    let early = super::worker::Worker::spawn(
        String::from("x"),
        root.to_path_buf(),
        settings,
        Isolation::Host,
        true,
    )?;
    drop(early);
    Ok(())
}

/// A host that cannot confine reaches the frame as one typed failure: no question, no copy,
/// and the person is told why (larry#243).
#[test]
fn sim_tui_worker_refused_confinement_ends_failed_with_the_reason()
-> Result<(), Box<dyn std::error::Error>> {
    let stale = crate::executor::sim_tree::leftover("larry-tui-refused", "ws")?;
    assert!(stale.join("ws").exists(), "the old leftover is present");
    let root = crate::executor::sim_tree::scratch("tui-refused")?;
    let settings = unreachable_model(&root)?;
    let isolation = Isolation::Workspace(crate::executor::Platform::Other);
    let worker = super::worker::Worker::spawn(
        String::from("x"),
        root.to_path_buf(),
        settings,
        isolation,
        false,
    )?;
    let seen = every_event(&worker).ok_or("the session sent no event inside its bound")?;
    match seen.last() {
        Some(&Event::Ended(Ended::Failed(ref why))) => {
            assert!(why.contains("no kernel confinement"), "{why}");
        }
        other => return Err(format!("expected a typed failure, got {other:?}").into()),
    }
    assert!(
        !seen
            .iter()
            .any(|event| matches!(*event, Event::Ask(_) | Event::Review(_)))
    );
    assert!(
        !root.join("ws").exists(),
        "a refused session copied nothing"
    );
    Ok(())
}

/// `/status` names the reason commands are off, whatever it is, and says commands run in an
/// isolated copy when they may run.
#[test]
fn sim_tui_header_status_names_why_commands_are_off() -> Result<(), Box<dyn std::error::Error>> {
    let stale = crate::executor::sim_tree::leftover("larry-tui-header", "state")?;
    assert!(stale.join("state").exists(), "the old leftover is present");
    let root = crate::executor::sim_tree::scratch("tui-header")?;
    let state = root.to_string_lossy().into_owned();
    let with = |policy: Option<&'static str>| {
        let state = state.clone();
        crate::executor::Settings::from_env(move |name| match name {
            "LARRY_STATE_DIR" => Some(state.clone()),
            "LARRY_POLICY" => policy.map(str::to_owned),
            _ => None,
        })
    };
    let (_, off) = super::header(&with(Some("")), &root, None);
    let line = off
        .iter()
        .find(|line| line.starts_with("commands:"))
        .ok_or("no commands line")?;
    assert!(
        line.starts_with("commands: off (the policy does not allow a session"),
        "{line}"
    );
    let (_, on) = super::header(&with(None), &root, None);
    let line = on
        .iter()
        .find(|line| line.starts_with("commands:"))
        .ok_or("no commands line")?;
    assert_eq!(
        line,
        "commands: asked before each one, run in an isolated copy"
    );
    Ok(())
}

/// A frame holding a review answers it only with an explicit yes: every key is judged against
/// the rule, and "always" has no meaning for a change to the person's own folder.
#[test]
fn sim_tui_apply_or_discard_answers_the_review() {
    use super::app::Effect;
    use super::worker::Decision;
    let summary = vec![
        String::from("1 file changed, +1 -0"),
        String::from("a.txt  new  +1 -0"),
    ];
    for seed in 0..SEEDS {
        let mut rng = Seeded::new(seed);
        let mut app = App::new(header(), Vec::new(), 40 + rng.pick(200), 10 + rng.pick(60));
        app.handle(Input::Key(Key::Char('x')));
        app.handle(Input::Key(Key::Enter));
        app.handle(Input::Worker(Event::Review(summary.clone())));
        assert!(app.busy(), "seed {seed}");
        let (key, want) = match rng.pick(9) {
            0 => (Key::Char('y'), Some(Decision::Run)),
            1 => (Key::Char('Y'), Some(Decision::Run)),
            2 => (Key::Char('n'), Some(Decision::Decline)),
            3 => (Key::Esc, Some(Decision::Decline)),
            4 => (Key::Enter, Some(Decision::Decline)),
            5 => (Key::Interrupt, Some(Decision::Decline)),
            6 => (Key::Char('a'), None),
            7 => (Key::Char('q'), None),
            _ => (Key::Up, None),
        };
        let effects = app.handle(Input::Key(key));
        match want {
            Some(decision) => {
                assert_eq!(
                    effects.first(),
                    Some(&Effect::Decide(decision)),
                    "seed {seed} {key:?}"
                );
            }
            None => assert!(
                effects.is_empty(),
                "seed {seed} {key:?} answered the review"
            ),
        }
    }
}

/// The toggle moves between Apply and Discard only, and Enter confirms the highlighted one.
#[test]
fn sim_tui_apply_toggle_confirms_the_highlight() {
    use super::app::Effect;
    use super::worker::Decision;
    let mut app = App::new(header(), Vec::new(), 100, 30);
    app.handle(Input::Worker(Event::Review(vec![String::from(
        "1 file changed, +1 -0",
    )])));
    assert!(app.handle(Input::Key(Key::Left)).is_empty());
    assert!(app.handle(Input::Key(Key::Right)).is_empty());
    let effects = app.handle(Input::Key(Key::Left));
    assert!(effects.is_empty());
    let confirmed = app.handle(Input::Key(Key::Enter));
    assert_eq!(confirmed.first(), Some(&Effect::Decide(Decision::Run)));
}

/// Names in a review and the hand-off line are sanitized in `sim_parts`.
#[test]
fn tui_error_variants_name_their_cause() {
    use std::error::Error as _;
    let terminal = super::TuiError::Terminal(std::io::Error::other("no tty"));
    assert_eq!(terminal.to_string(), "terminal: no tty");
    assert!(terminal.source().is_some(), "the io cause is kept");
    let directory = super::TuiError::Directory(std::io::Error::other("gone"));
    assert_eq!(directory.to_string(), "working directory: gone");
    assert!(directory.source().is_some(), "the io cause is kept");
}

/// What Up recalls from an empty composer, newest first, until it stops moving.
fn recalled(composer: &mut super::composer::Composer) -> Vec<String> {
    let mut seen: Vec<String> = Vec::new();
    for _ in 0..=super::composer::HISTORY {
        let before = composer.text().to_owned();
        composer.up();
        let text = composer.text().to_owned();
        if text == before {
            break;
        }
        seen.push(text);
    }
    seen
}

/// The double-clicker and the fat-finger: blank and repeated submissions. Reference model:
/// history keeps a submission only when it is not blank and not the same as the newest kept
/// one, so Up never offers an empty line or the same line twice in a row.
#[test]
fn sim_tui_history_skips_blank_and_repeated_submissions() {
    let pool = [
        "",
        "   ",
        "\n",
        "fix the build",
        "fix the build",
        "run tests",
        "ls",
    ];
    for seed in 0..SEEDS {
        let mut rng = Seeded::new(seed);
        let mut composer = super::composer::Composer::default();
        let mut model: Vec<String> = Vec::new();
        for _ in 0..rng.pick(24) {
            let text = rng.word(&pool);
            composer.insert(text);
            assert_eq!(composer.take(), sanitize_kept(text));
            let kept = sanitize_kept(text);
            if !kept.trim().is_empty() && model.last() != Some(&kept) {
                model.push(kept);
            }
        }
        let mut expected = model.clone();
        expected.reverse();
        assert_eq!(recalled(&mut composer), expected, "seed {seed}");
    }
}

/// The text the composer keeps for `text`: newlines stay, controls go.
fn sanitize_kept(text: &str) -> String {
    super::style::sanitize(text, true)
}

/// The wrong-order fool: Up on the second line of a draft moves within the draft, never
/// replaces it with history; Up on the first line recalls.
#[test]
fn sim_tui_up_recalls_history_only_from_the_first_line() {
    let mut composer = super::composer::Composer::default();
    composer.insert("earlier task");
    composer.take();
    composer.insert("first line\nsecond line");
    composer.up();
    assert_eq!(
        composer.text(),
        "first line\nsecond line",
        "the draft is kept"
    );
    composer.home();
    composer.clear();
    composer.insert("one line");
    composer.up();
    assert_eq!(composer.text(), "earlier task", "the first line recalls");
}

/// Only `bold()` makes a style bold: every tone alone writes reset then its colour.
#[test]
fn sim_tui_plain_styles_are_never_bold() {
    let cases: [(Style, &[u8]); 6] = [
        (Style::PLAIN, b"\x1b[0m"),
        (Style::MUTED, b"\x1b[0;2m"),
        (Style::ACCENT, b"\x1b[0;36m"),
        (Style::GOOD, b"\x1b[0;32m"),
        (Style::WARN, b"\x1b[0;33m"),
        (Style::BAD, b"\x1b[0;31m"),
    ];
    for (style, wire) in cases {
        let mut out = Vec::new();
        style.sgr(&mut out);
        assert_eq!(out, wire);
    }
}

/// The giant-and-empty fool: seeded words and runs of spaces wrapped at every width. Oracle:
/// no row starts with a space, no row is wider than the width (a word wider than a row is cut
/// by character), and the words read back in order.
#[test]
fn sim_tui_wrap_drops_spaces_at_breaks_and_keeps_every_word() {
    let words = ["a", "bb", "cccc", "ddddddddd", "表示", "e\u{301}"];
    for seed in 0..SEEDS {
        let mut rng = Seeded::new(seed);
        let mut text = String::new();
        for _ in 0..rng.pick(16) {
            for _ in 0..rng.pick(4) {
                text.push(' ');
            }
            text.push_str(rng.word(&words));
        }
        let width = 2 + rng.pick(20);
        let rows = super::style::wrap(
            &[super::style::Span::new(text.clone(), Style::PLAIN)],
            width,
            &[],
            &[],
        );
        let mut read = String::new();
        for row in &rows {
            let joined: String = row.iter().map(|span| span.text.as_str()).collect();
            assert!(
                !joined.starts_with(' '),
                "seed {seed} width {width}: {joined:?}"
            );
            assert!(
                super::style::text_width(&joined) <= width,
                "seed {seed}: {joined:?}"
            );
            read.push_str(&joined);
        }
        let squeeze = |value: &str| value.split_whitespace().collect::<String>();
        assert_eq!(squeeze(&read), squeeze(&text), "seed {seed} width {width}");
    }
}
