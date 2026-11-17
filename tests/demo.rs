//! Integration journeys for the fixture domain: the published JSON view, the
//! rejection contract, and session isolation. These drive `larry_demo`
//! directly; the shipped driver has `tests/demo_journey.rs` and the native
//! surface its real-PTY suite in `tests/e2e_journey.rs`.

use larry_demo::{Demo, MAX_INPUT, Rejected};
use lgwks_std::json::{self, Value};

/// Read the public snapshot, propagating either serialization failure.
fn view(demo: &Demo) -> Result<Value, json::Error> {
    json::from_str(&demo.snapshot()?)
}

#[test]
fn quiet_default_and_real_estate_serialization() -> Result<(), Box<dyn std::error::Error>> {
    let snapshot = view(&Demo::default())?;
    assert!(
        snapshot["mode"]
            .as_str()
            .ok_or("mode must be a string")?
            .starts_with("DEMO")
    );
    assert_eq!(
        snapshot["events"]
            .as_array()
            .ok_or("events must be an array")?
            .len(),
        0
    );
    assert!(snapshot["agents"].is_null());
    assert!(snapshot["folders"].is_null());
    Ok(())
}

#[test]
fn hold_remembers_original_anchor_while_other_owner_advances()
-> Result<(), Box<dyn std::error::Error>> {
    let mut demo = Demo::default();
    for command in ["next", "feedback bound retries", "next", "inspect 2"] {
        assert!(demo.command(command)?);
    }
    let held = demo.clone();
    assert_eq!(
        demo.command("feedback replace hold"),
        Err(Rejected::HeldWork)
    );
    assert_eq!(demo, held);
    assert_eq!(demo.command("next"), Err(Rejected::HeldWork));
    assert_eq!(demo, held);
    assert_eq!(view(&demo)?["work"]["Held"]["anchor"], 1);
    assert_eq!(view(&demo)?["inspection"]["trace"], "fixture-02");
    assert!(demo.command("continue")?);
    assert!(demo.command("next")?);
    assert_eq!(view(&demo)?["inspection"]["line"], 18);
    assert_eq!(view(&demo)?["inspection"]["file"], "src/retry.rs");
    assert_eq!(view(&demo)?["inspection"]["before"], "sleep(delay)");
    assert_eq!(
        view(&demo)?["inspection"]["after"],
        "sleep(delay.min(remaining))"
    );
    assert_eq!(view(&demo)?["feedback"][0]["text"], "bound retries");
    assert_eq!(demo.command("continue"), Err(Rejected::NoHeldWork));
    Ok(())
}

#[test]
fn invalid_inputs_are_atomic_and_control_text_is_rejected() {
    let mut demo = Demo::default();
    for command in [
        "inspect 0",
        "inspect 1",
        "inspect 9999999999999999999999999",
        "feedback hi",
        "feedback",
        "feedbackx",
        "exec touch anything",
        "\u{1b}[2J",
        "x\nnext",
    ] {
        assert!(demo.command(command).is_err(), "{command:?}");
        assert_eq!(demo, Demo::default());
    }
    assert_eq!(
        demo.command(&"x".repeat(MAX_INPUT.saturating_add(1))),
        Err(Rejected::InputBound)
    );
}

#[test]
fn toggles_are_reversible_and_clone_tree_has_one_parent() -> Result<(), Box<dyn std::error::Error>>
{
    let mut demo = Demo::default();
    assert!(demo.command("folders")?);
    assert!(demo.command("agents")?);
    let snapshot = view(&demo)?;
    assert_eq!(
        snapshot["folders"]
            .as_array()
            .ok_or("folders must be an array")?
            .len(),
        2
    );
    let agents = snapshot["agents"]
        .as_array()
        .ok_or("agents must be an array")?;
    assert!(agents[0]["parent"].is_null());
    assert_eq!(agents[1]["parent"], 0);
    assert_eq!(agents[2]["parent"], 0);
    assert!(demo.command("folders")?);
    assert!(demo.command("agents")?);
    assert_eq!(demo, Demo::default());
    Ok(())
}

#[test]
fn empty_reset_end_and_quit_are_explicit() -> Result<(), Box<dyn std::error::Error>> {
    let mut demo = Demo::default();
    assert!(demo.command("scenario empty")?);
    assert_eq!(demo.command("next"), Err(Rejected::EndOfFixture));
    assert_eq!(view(&demo)?["scenario"], "empty");
    assert!(demo.command("scenario change")?);
    for _ in 0..3 {
        assert!(demo.command("next")?);
    }
    assert_eq!(demo.command("next"), Err(Rejected::EndOfFixture));
    assert!(demo.command("reset")?);
    assert_eq!(demo, Demo::default());
    assert!(!demo.command("quit")?);
    Ok(())
}

#[test]
fn unicode_notes_have_finite_capacity_and_other_sessions_are_isolated()
-> Result<(), Box<dyn std::error::Error>> {
    let mut demo = Demo::default();
    let other = Demo::default();
    assert!(demo.command("next")?);
    for _ in 0..16 {
        assert!(demo.command("feedback préserver 日本語")?);
        assert!(demo.command("continue")?);
    }
    let before = demo.clone();
    assert_eq!(demo.command("feedback overflow"), Err(Rejected::NoteBound));
    assert_eq!(demo, before);
    assert_eq!(other, Demo::default());
    assert_eq!(
        view(&demo)?["feedback"]
            .as_array()
            .ok_or("feedback must be an array")?
            .len(),
        16
    );
    assert_eq!(view(&demo)?["feedback"][0]["text"], "préserver 日本語");
    Ok(())
}
