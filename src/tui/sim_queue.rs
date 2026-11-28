//! Seeded simulations of queued turns and background commands (larry#262, ADR 0128):
//! Enter while a task runs queues the next turn, the queue paints above the
//! composer, and each `Ended` starts the next queued turn in order. One seed
//! drives every key and session event; no clock, no terminal.

use super::app::{App, Effect, Input, Key};
use super::cells::OUTPUT_TAIL;
use super::pending::{JOBS_BOUND, Pending};
use super::sim_parts::{header, shown};
use super::worker::{Decision, Ended, Event};

/// An empty frame at the simulation size.
fn fresh_app() -> App {
    App::new(header(), Vec::new(), 120, 34)
}

/// Types `first`, sends it and reports its `sleep 30` started.
fn start_first(app: &mut App) {
    for character in "first".chars() {
        app.handle(Input::Key(Key::Char(character)));
    }
    app.handle(Input::Key(Key::Enter));
    app.handle(Input::Worker(Event::Started(String::from("sleep 30"))));
}

/// Types `text` and presses Enter: one slash command or, while a session runs,
/// one queued turn.
fn enter(app: &mut App, text: &str) {
    for character in text.chars() {
        app.handle(Input::Key(Key::Char(character)));
    }
    app.handle(Input::Key(Key::Enter));
}

/// Queued turns send in order: typing Enter while a task runs queues
/// the text for the next turn, and the next queued turn starts when the
/// running one ends.
#[test]
fn sim_queued_tasks_send_in_order() {
    let mut app = fresh_app();
    for character in "first".chars() {
        app.handle(Input::Key(Key::Char(character)));
    }
    let started = app.handle(Input::Key(Key::Enter));
    assert_eq!(
        started,
        vec![Effect::Start(String::from("first"))],
        "the first task starts"
    );
    for character in "second".chars() {
        app.handle(Input::Key(Key::Char(character)));
    }
    let queued = app.handle(Input::Key(Key::Enter));
    assert!(
        queued.is_empty(),
        "Enter while working queues, it never starts: {queued:?}"
    );
    for character in "third".chars() {
        app.handle(Input::Key(Key::Char(character)));
    }
    let queued_again = app.handle(Input::Key(Key::Enter));
    assert!(
        queued_again.is_empty(),
        "a second Enter queues behind the first: {queued_again:?}"
    );
    let painted = shown(&app);
    assert!(
        painted.contains("second") && painted.contains("third"),
        "the queue shows above the composer: {painted:?}"
    );
    let next = app.handle(Input::Worker(Event::Ended(Ended::Done)));
    assert_eq!(
        next,
        vec![Effect::Start(String::from("second"))],
        "the first queued turn starts when the run ends"
    );
    let then = app.handle(Input::Worker(Event::Ended(Ended::Done)));
    assert_eq!(
        then,
        vec![Effect::Start(String::from("third"))],
        "queued turns send in order"
    );
}

/// `/ps` lists background commands with their last lines: a started
/// command lists as running, and after it ends its tail shows. One command
/// reads singular, two read plural.
#[test]
fn sim_ps_lists_background_commands_with_last_lines() {
    let mut app = fresh_app();
    start_first(&mut app);
    for character in "/ps".chars() {
        app.handle(Input::Key(Key::Char(character)));
    }
    assert!(
        app.handle(Input::Key(Key::Enter)).is_empty(),
        "/ps answers in the transcript, it never starts or stops"
    );
    let running = shown(&app);
    assert!(
        running.contains("1 background command:") && running.contains("running $ sleep 30"),
        "the running command lists, singular: {running:?}"
    );
    app.handle(Input::Worker(Event::Started(String::from("sleep 31"))));
    for character in "/ps".chars() {
        app.handle(Input::Key(Key::Char(character)));
    }
    app.handle(Input::Key(Key::Enter));
    assert!(
        shown(&app).contains("2 background commands:"),
        "two commands read plural"
    );
    app.handle(Input::Worker(Event::Finished {
        observation: String::from("exit status 0\nstdout:\nhello\nworld\nstderr:\n"),
        passed: true,
    }));
    for character in "/ps".chars() {
        app.handle(Input::Key(Key::Char(character)));
    }
    app.handle(Input::Key(Key::Enter));
    let ended = shown(&app);
    assert!(
        ended.contains("hello") && ended.contains("world"),
        "the last lines list: {ended:?}"
    );
    assert!(
        ended.contains("2: done $ sleep 31"),
        "the finished command reads done: {ended:?}"
    );
    assert!(
        ended.contains("running $ sleep 30"),
        "the other command still runs: {ended:?}"
    );
}

/// A finish with no running command still lists, as ended with its tail.
#[test]
fn sim_finish_without_a_start_lists_as_ended() {
    let mut app = fresh_app();
    for character in "first".chars() {
        app.handle(Input::Key(Key::Char(character)));
    }
    app.handle(Input::Key(Key::Enter));
    app.handle(Input::Worker(Event::Finished {
        observation: String::from("exit status 1\nstdout:\nnope\nstderr:\n"),
        passed: false,
    }));
    for character in "/ps".chars() {
        app.handle(Input::Key(Key::Char(character)));
    }
    app.handle(Input::Key(Key::Enter));
    let listed = shown(&app);
    assert!(
        listed.contains("ended $") && listed.contains("nope"),
        "the finish lists with its tail: {listed:?}"
    );
    assert!(!listed.contains("running"), "nothing runs: {listed:?}");
}

/// `/stop` ends the running command through the session's cancel path,
/// the same one Esc uses; with nothing running it only says so.
#[test]
fn sim_stop_ends_the_running_command() {
    let mut app = fresh_app();
    for character in "/stop".chars() {
        app.handle(Input::Key(Key::Char(character)));
    }
    assert!(
        app.handle(Input::Key(Key::Enter)).is_empty(),
        "idle /stop stops nothing"
    );
    assert!(
        shown(&app).contains("No background command"),
        "idle /stop says so"
    );
    start_first(&mut app);
    for character in "/stop".chars() {
        app.handle(Input::Key(Key::Char(character)));
    }
    let stop = app.handle(Input::Key(Key::Enter));
    assert_eq!(
        stop,
        vec![Effect::Interrupt],
        "/stop interrupts the session"
    );
    let done = app.handle(Input::Worker(Event::Ended(Ended::Interrupted)));
    assert!(done.is_empty(), "no queued turn starts after an interrupt");
    assert!(
        shown(&app).contains("Interrupted"),
        "the interrupt is painted"
    );
}

/// `/quit` with a running background command asks first and ends it, then quits
/// when the session ends. With nothing running while busy it leaves at once.
#[test]
fn sim_quit_with_running_jobs_asks_first() {
    let mut app = fresh_app();
    start_first(&mut app);
    for character in "/quit".chars() {
        app.handle(Input::Key(Key::Char(character)));
    }
    let first = app.handle(Input::Key(Key::Enter));
    assert_eq!(
        first,
        vec![Effect::Interrupt],
        "the first /quit ends the command, it never leaves at once"
    );
    let quit = app.handle(Input::Worker(Event::Ended(Ended::Interrupted)));
    assert_eq!(quit, vec![Effect::Quit], "quit follows the session's end");

    let mut idle = fresh_app();
    for character in "first".chars() {
        idle.handle(Input::Key(Key::Char(character)));
    }
    idle.handle(Input::Key(Key::Enter));
    for character in "/quit".chars() {
        idle.handle(Input::Key(Key::Char(character)));
    }
    assert_eq!(
        idle.handle(Input::Key(Key::Enter)),
        vec![Effect::Quit],
        "with no running command /quit leaves at once"
    );
}

/// The interrupt's quit hint names the count: none while thinking, two with two
/// running commands.
#[test]
fn sim_interrupt_quit_hint_names_the_running_count() {
    let mut thinking = fresh_app();
    for character in "first".chars() {
        thinking.handle(Input::Key(Key::Char(character)));
    }
    thinking.handle(Input::Key(Key::Enter));
    assert_eq!(
        thinking.handle(Input::Key(Key::Esc)),
        vec![Effect::Interrupt],
        "Esc stops the thinking session"
    );
    assert!(
        thinking.handle(Input::Key(Key::Interrupt)).is_empty(),
        "the second Ctrl+C only arms the quit"
    );
    let plain = shown(&thinking);
    assert!(
        plain.contains("Larry quits when the session ends.")
            && !plain.contains("background command"),
        "no count with nothing running: {plain:?}"
    );

    let mut two = fresh_app();
    start_first(&mut two);
    two.handle(Input::Worker(Event::Started(String::from("sleep 31"))));
    assert_eq!(
        two.handle(Input::Key(Key::Esc)),
        vec![Effect::Interrupt],
        "Esc stops the session"
    );
    two.handle(Input::Key(Key::Interrupt));
    assert!(
        shown(&two).contains("2 background commands running"),
        "two commands read plural"
    );
}

/// Answering `a` runs this command and every later one without asking again.
#[test]
fn sim_answer_always_runs_and_stops_asking() {
    let mut app = fresh_app();
    for character in "first".chars() {
        app.handle(Input::Key(Key::Char(character)));
    }
    app.handle(Input::Key(Key::Enter));
    app.handle(Input::Worker(Event::Ask(String::from("rm -rf /tmp/x"))));
    app.draw();
    assert_eq!(
        app.handle(Input::Key(Key::Char('a'))),
        vec![Effect::Decide(Decision::Always)],
        "`a` approves without asking again"
    );
    assert!(app.always(), "the person said to stop asking");
}

/// The queue holds eight turns: the ninth is refused and never sends.
#[test]
fn sim_queue_bound_refuses_past_eight() {
    let mut app = fresh_app();
    for character in "first".chars() {
        app.handle(Input::Key(Key::Char(character)));
    }
    app.handle(Input::Key(Key::Enter));
    for index in 0..8 {
        let task = format!("task{index}");
        for character in task.chars() {
            app.handle(Input::Key(Key::Char(character)));
        }
        let queued = app.handle(Input::Key(Key::Enter));
        assert!(
            queued.is_empty(),
            "queued turn {index} never starts at once: {queued:?}"
        );
    }
    for character in "ninth".chars() {
        app.handle(Input::Key(Key::Char(character)));
    }
    let refused = app.handle(Input::Key(Key::Enter));
    assert!(
        refused.is_empty(),
        "a full queue starts nothing: {refused:?}"
    );
    assert!(shown(&app).contains("full (8)"), "the bound is named");
    for index in 0..8 {
        let next = app.handle(Input::Worker(Event::Ended(Ended::Done)));
        assert_eq!(
            next,
            vec![Effect::Start(format!("task{index}"))],
            "queued turn {index} sends in order"
        );
    }
    let last = app.handle(Input::Worker(Event::Ended(Ended::Done)));
    assert!(
        last.is_empty(),
        "the refused ninth turn never sends: {last:?}"
    );
}

/// The session's end marks every running command done: `/ps` after `Ended`
/// lists it as ended, never running (larry#262).
#[test]
fn sim_ended_session_lists_no_running() {
    let mut app = fresh_app();
    start_first(&mut app);
    app.handle(Input::Worker(Event::Ended(Ended::Done)));
    enter(&mut app, "/ps");
    let listed = shown(&app);
    assert!(
        listed.contains("ended $ sleep 30"),
        "the ended command lists as ended: {listed:?}"
    );
    assert!(
        !listed.contains("running $"),
        "no running row lists after the end: {listed:?}"
    );
}

/// `/quit` with one running command names it singular: the plural branch stays
/// for two (larry#262).
#[test]
fn sim_quit_hint_reads_singular_with_one_job() {
    let mut app = fresh_app();
    start_first(&mut app);
    enter(&mut app, "/quit");
    let hinted = shown(&app);
    assert!(
        hinted.contains("1 background command running."),
        "one command reads singular: {hinted:?}"
    );
    assert!(
        !hinted.contains("commands running"),
        "singular never reads plural: {hinted:?}"
    );
}

/// Past sixteen commands the oldest drops: the seventeenth push keeps sixteen
/// with the newest last (larry#262).
#[test]
fn sim_jobs_bound_keeps_newest_and_drops_oldest() {
    let mut pending = Pending::new();
    for index in 0..JOBS_BOUND + 1 {
        pending.push_job(&format!("job-{index}"));
    }
    let rows = pending.ps_rows().join("\n");
    assert!(
        rows.contains("16 background commands:"),
        "the bound holds sixteen: {rows:?}"
    );
    assert!(
        !rows.contains("job-0"),
        "the oldest drops past the bound: {rows:?}"
    );
    assert!(rows.contains("job-16"), "the newest stays: {rows:?}");
    assert_eq!(
        pending.running_jobs(),
        JOBS_BOUND,
        "every kept command still runs"
    );
}

/// Only the last lines list: a finish with more than five output lines keeps
/// the last five and drops the rest (larry#262).
#[test]
fn sim_output_tail_lists_only_last_lines() {
    let mut pending = Pending::new();
    pending.push_job("sleep 30");
    let output: Vec<String> = (0..OUTPUT_TAIL + 2)
        .map(|index| format!("line-{index}"))
        .collect();
    pending.finish_job("sleep 30", &output, true);
    let rows = pending.ps_rows().join("\n");
    assert!(
        rows.contains("done $ sleep 30"),
        "the finish reads done: {rows:?}"
    );
    for index in 2..OUTPUT_TAIL + 2 {
        assert!(
            rows.contains(&format!("line-{index}")),
            "the kept tail holds line-{index}: {rows:?}"
        );
    }
    assert!(
        !rows.contains("line-0") && !rows.contains("line-1"),
        "lines past the tail drop: {rows:?}"
    );
}

/// Only the first line shows: a multi-line command lists as its first line in
/// `/ps`, the queue and the status row (larry#262).
#[test]
fn sim_first_line_hides_later_lines() {
    let mut pending = Pending::new();
    pending.push_job("sleep 30\nrm -rf /");
    let rows = pending.ps_rows().join("\n");
    assert!(
        rows.contains("running $ sleep 30"),
        "the first line lists: {rows:?}"
    );
    assert!(!rows.contains("rm -rf"), "later lines never list: {rows:?}");

    let mut app = fresh_app();
    enter(&mut app, "first");
    app.handle(Input::Worker(Event::Started(String::from(
        "sleep 30\nrm -rf /",
    ))));
    enter(&mut app, "/ps");
    let listed = shown(&app);
    assert!(
        listed.contains("running $ sleep 30"),
        "the frame lists one row: {listed:?}"
    );
    assert!(
        !listed.contains("rm -rf"),
        "the frame hides later lines: {listed:?}"
    );
    assert!(
        shown(&app).contains("Running sleep 30"),
        "the status row hides them too: {listed:?}"
    );
}

/// Queue rows number in order: the first waiting turn reads `queued 1` and the
/// next `queued 2` (larry#262).
#[test]
fn sim_queue_rows_number_in_order() {
    let mut app = fresh_app();
    enter(&mut app, "first");
    enter(&mut app, "second");
    enter(&mut app, "third");
    let painted = shown(&app);
    assert!(
        painted.contains("queued 1: second"),
        "the first waiter reads one: {painted:?}"
    );
    assert!(
        painted.contains("queued 2: third"),
        "the next waiter reads two: {painted:?}"
    );
}

/// Queued turns drain through the pending queue: pushing past the bound keeps
/// the first eight and drops the ninth, oldest first.
#[test]
fn sim_pending_queue_holds_eight_oldest_first() {
    let mut pending = Pending::new();
    assert!(!pending.queue_full(), "an empty queue takes a turn");
    for index in 0..8 {
        pending.push_turn(format!("task{index}"));
    }
    assert!(pending.queue_full(), "eight turns fill the queue");
    pending.push_turn(String::from("ninth"));
    for index in 0..8 {
        assert_eq!(
            pending.pop_next(),
            Some(format!("task{index}")),
            "turn {index} waits in order"
        );
    }
    assert_eq!(pending.pop_next(), None, "the ninth never queued");
    pending.clear_queue();
    assert!(!pending.queue_full(), "quitting clears the queue");
}

/// `/stop` of a job that already ended only says so: nothing runs, so no
/// interrupt leaves the frame (larry#556).
#[test]
fn sim_stop_after_finish_says_idle() {
    let mut app = fresh_app();
    start_first(&mut app);
    app.handle(Input::Worker(Event::Finished {
        observation: String::from("exit status 0\nstdout:\nhello\nstderr:\n"),
        passed: true,
    }));
    enter(&mut app, "/stop");
    let listed = shown(&app);
    assert!(
        listed.contains("No background command is running."),
        "an ended job leaves /stop idle: {listed:?}"
    );
}

/// `/stop` then the job's own `Finished`: the stop races the exit, and the
/// frame still ends quiet with the job listed as ended, never running
/// (larry#556).
#[test]
fn sim_stop_race_with_finish_ends_quiet() {
    let mut app = fresh_app();
    start_first(&mut app);
    enter(&mut app, "/stop");
    let again = {
        for character in "/stop".chars() {
            app.handle(Input::Key(Key::Char(character)));
        }
        app.handle(Input::Key(Key::Enter))
    };
    assert!(
        again.is_empty(),
        "a second /stop sends no second interrupt: {again:?}"
    );
    app.handle(Input::Worker(Event::Finished {
        observation: String::from("exit status 130\nstdout:\npartial\nstderr:\n"),
        passed: false,
    }));
    let done = app.handle(Input::Worker(Event::Ended(Ended::Interrupted)));
    assert!(done.is_empty(), "no queued turn starts after an interrupt");
    enter(&mut app, "/ps");
    let listed = shown(&app);
    assert!(
        listed.contains("ended $ sleep 30"),
        "the raced job lists as ended: {listed:?}"
    );
    assert!(
        listed.contains("partial"),
        "the raced finish keeps its tail: {listed:?}"
    );
    assert!(
        !listed.contains("running $"),
        "no running row lists after the race: {listed:?}"
    );
}

/// A finish before `/stop` leaves `/stop` idle too: the command ended on its
/// own while the frame stayed working (larry#556).
#[test]
fn sim_finish_then_stop_says_idle() {
    let mut app = fresh_app();
    start_first(&mut app);
    app.handle(Input::Worker(Event::Finished {
        observation: String::from("exit status 0\nstdout:\n\nstderr:\n"),
        passed: true,
    }));
    for character in "/stop".chars() {
        app.handle(Input::Key(Key::Char(character)));
    }
    assert!(
        app.handle(Input::Key(Key::Enter)).is_empty(),
        "/stop after a finish stops nothing"
    );
    assert!(
        shown(&app).contains("No background command is running."),
        "the frame says so"
    );
}

/// `/ps` past its row bound stays bounded: seventeen starts keep sixteen
/// jobs, each with at most five lines, so the rows never grow past one
/// header plus sixteen commands with their tails (larry#556).
#[test]
fn sim_ps_rows_stay_within_bound() {
    let mut pending = Pending::new();
    for index in 0..JOBS_BOUND + 1 {
        pending.push_job(&format!("job-{index}"));
        let output: Vec<String> = (0..OUTPUT_TAIL + 2)
            .map(|line| format!("line-{line}"))
            .collect();
        pending.finish_job(&format!("job-{index}"), &output, true);
    }
    let rows = pending.ps_rows();
    let cap = 1 + JOBS_BOUND * (1 + OUTPUT_TAIL);
    assert!(
        rows.len() <= cap,
        "{} rows pass the bound of {cap}: {rows:?}",
        rows.len()
    );
    let joined = rows.join("\n");
    assert!(
        !joined.contains("job-0"),
        "the oldest drops past the bound: {joined:?}"
    );
    assert!(joined.contains("done $"), "kept jobs read done");
}

/// A job whose output holds escapes lists sanitized: no escape byte reaches
/// the transcript, and the text stays readable (larry#556).
#[test]
fn sim_ps_sanitizes_hostile_output() {
    let mut pending = Pending::new();
    pending.push_job("sleep 30");
    pending.finish_job(
        "sleep 30",
        &[
            String::from("\u{1b}[31mred\u{1b}[0m"),
            String::from("bell\u{7}here"),
        ],
        true,
    );
    let joined = pending.ps_rows().join("\n");
    assert!(
        !joined.contains('\u{1b}'),
        "no escape byte lists: {joined:?}"
    );
    assert!(!joined.contains('\u{7}'), "no bell lists: {joined:?}");
    assert!(
        joined.contains("red") && joined.contains("bellhere"),
        "the text stays readable: {joined:?}"
    );
}
