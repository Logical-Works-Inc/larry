//! Seeded simulations of queued turns (larry#262, ADR 0128): Enter while a
//! task runs queues the next turn, the queue paints above the composer, and
//! each `Ended` starts the next queued turn in order. One seed drives every
//! key and session event; no clock, no terminal.

use super::app::{App, Effect, Input, Key};
use super::pending::Pending;
use super::sim_parts::{header, shown};
use super::worker::{Decision, Ended, Event};

/// An empty frame at the simulation size.
fn fresh_app() -> App {
    App::new(header(), Vec::new(), 120, 34)
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
