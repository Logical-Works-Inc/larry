use lgwks_std::json::{self, Value};
use rocco_demo::{Demo, MAX_INPUT, Rejected};

fn view(demo: &Demo) -> Value {
    json::from_str(&demo.snapshot().unwrap()).unwrap()
}

#[test]
fn quiet_default_and_real_estate_serialization() {
    let v = view(&Demo::default());
    assert!(v["mode"].as_str().unwrap().starts_with("DEMO"));
    assert_eq!(v["events"].as_array().unwrap().len(), 0);
    assert!(v["agents"].is_null());
    assert!(v["folders"].is_null());
}

#[test]
fn hold_remembers_original_anchor_while_other_owner_advances() {
    let mut d = Demo::default();
    for c in ["next", "feedback bound retries", "next", "inspect 2"] {
        d.command(c).unwrap();
    }
    let held = d.clone();
    assert_eq!(d.command("feedback replace hold"), Err(Rejected::HeldWork));
    assert_eq!(d, held);
    assert_eq!(d.command("next"), Err(Rejected::HeldWork));
    assert_eq!(d, held);
    assert_eq!(view(&d)["work"]["Held"]["anchor"], 1);
    assert_eq!(view(&d)["inspection"]["trace"], "fixture-02");
    d.command("continue").unwrap();
    d.command("next").unwrap();
    assert_eq!(view(&d)["inspection"]["line"], 18);
    assert_eq!(view(&d)["inspection"]["file"], "src/retry.rs");
    assert_eq!(view(&d)["inspection"]["before"], "sleep(delay)");
    assert_eq!(
        view(&d)["inspection"]["after"],
        "sleep(delay.min(remaining))"
    );
    assert_eq!(view(&d)["feedback"][0]["text"], "bound retries");
    assert_eq!(d.command("continue"), Err(Rejected::NoHeldWork));
}

#[test]
fn invalid_inputs_are_atomic_and_control_text_is_rejected() {
    let mut d = Demo::default();
    for c in [
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
        assert!(d.command(c).is_err(), "{c:?}");
        assert_eq!(d, Demo::default());
    }
    assert_eq!(
        d.command(&"x".repeat(MAX_INPUT + 1)),
        Err(Rejected::InputBound)
    );
}

#[test]
fn toggles_are_reversible_and_clone_tree_has_one_parent() {
    let mut d = Demo::default();
    d.command("folders").unwrap();
    d.command("agents").unwrap();
    let v = view(&d);
    assert_eq!(v["folders"].as_array().unwrap().len(), 2);
    let agents = v["agents"].as_array().unwrap();
    assert!(agents[0]["parent"].is_null());
    assert_eq!(agents[1]["parent"], 0);
    assert_eq!(agents[2]["parent"], 0);
    d.command("folders").unwrap();
    d.command("agents").unwrap();
    assert_eq!(d, Demo::default());
}

#[test]
fn empty_reset_end_and_quit_are_explicit() {
    let mut d = Demo::default();
    d.command("scenario empty").unwrap();
    assert_eq!(d.command("next"), Err(Rejected::EndOfFixture));
    assert_eq!(view(&d)["scenario"], "empty");
    d.command("scenario change").unwrap();
    for _ in 0..3 {
        d.command("next").unwrap();
    }
    assert_eq!(d.command("next"), Err(Rejected::EndOfFixture));
    d.command("reset").unwrap();
    assert_eq!(d, Demo::default());
    assert!(!d.command("quit").unwrap());
}

#[test]
fn unicode_notes_have_finite_capacity_and_other_sessions_are_isolated() {
    let mut d = Demo::default();
    let other = Demo::default();
    d.command("next").unwrap();
    for _ in 0..16 {
        d.command("feedback préserver 日本語").unwrap();
        d.command("continue").unwrap();
    }
    let before = d.clone();
    assert_eq!(d.command("feedback overflow"), Err(Rejected::NoteBound));
    assert_eq!(d, before);
    assert_eq!(other, Demo::default());
    assert_eq!(view(&d)["feedback"].as_array().unwrap().len(), 16);
    assert_eq!(view(&d)["feedback"][0]["text"], "préserver 日本語");
}
