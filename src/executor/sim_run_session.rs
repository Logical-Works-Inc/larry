//! Whole terminal sessions through `run_session` against the scripted gateway
//! (larry#93, larry#566): the sealed report keeps what ran, what was spent and
//! why the session stopped; a session the gate denies runs nothing and says the
//! line it sealed; and a session refused before anything runs names its exact
//! typed error without dialling the model. Source: the Agent Client Protocol
//! prompt turn, CWE-20 (hostile input) and CWE-770 (nothing runs past a refused
//! gate).

use std::path::Path;

use lgwks_std::json::{Value, json};

use super::super::session_turn::SYSTEM;
use super::super::sim_gateway::{Cancel, Reply, Scripted, StubGateway, TestResult};
use super::super::sim_tree::scratch;
use super::super::{ExecError, Ledger, Platform, Settings};
use super::{
    Approval, Ended, Handoff, Isolation, MAX_INSTRUCTION_BYTES, Observer, SessionDoor,
    SessionError, Stop, authored_usage, check, run_session,
};
use crate::json_at::At;
use crate::model::{Draft, Kind, Route, Usage, tenant_namespace};

/// Every scope a terminal session on the host needs.
const HOST_POLICY: &str = "file.read,file.write,process.spawn,model.invoke,model.command";

/// The model key every settings here hold, a value no path or name repeats.
const SIM_KEY: &str = "sim-secret-never-printed";

/// Settings over `state` with `policy`, the model route pointed at `url` and
/// one attempt per completion.
fn settings(state: &Path, url: &str, policy: &str) -> TestResult<Settings> {
    let table: Vec<(&str, String)> = vec![
        (
            "LARRY_STATE_DIR",
            state
                .to_str()
                .ok_or("the state directory is not text")?
                .to_owned(),
        ),
        ("PATH", String::from("/usr/bin:/bin")),
        ("LARRY_POLICY", policy.to_owned()),
        ("LARRY_MODEL_URL", url.to_owned()),
        ("LARRY_MODEL", String::from("sim-run")),
        ("LARRY_MODEL_KEY", String::from(SIM_KEY)),
        ("LARRY_MODEL_ATTEMPTS", String::from("1")),
    ];
    Ok(Settings::from_env(move |name| {
        table
            .iter()
            .find(|entry| entry.0 == name)
            .map(|entry| entry.1.clone())
    })?)
}

/// What one whole session left behind.
struct Ran {
    /// How it ended, with its sealed report.
    ended: Ended,
    /// What the person saw and answered.
    observer: Scripted,
    /// Every request the gateway answered.
    requests: Vec<Value>,
}

/// Runs one host session of `replies`, the person answering as `observer`,
/// under `policy`, in a fresh state and work directory.
fn session(
    label: &str,
    replies: Vec<Reply>,
    mut observer: Scripted,
    policy: &str,
) -> TestResult<Ran> {
    let state = scratch(label)?;
    let work = scratch(label)?;
    let gateway = StubGateway::bind()?;
    let settings = settings(&state, gateway.url(), policy)?;
    let (ended, requests) = gateway.serve_while(replies, || {
        Ok(run_session(
            "a simulated session",
            &work,
            &settings,
            Isolation::Host,
            &mut observer,
            SessionDoor::Tenant,
        )?)
    })?;
    Ok(Ran {
        ended,
        observer,
        requests,
    })
}

/// The report member at `pointer`.
fn member<'a>(ran: &'a Ran, pointer: &str) -> TestResult<&'a Value> {
    Ok(ran.ended.report.json().at(pointer)?)
}

/// A session that runs one command and ends seals the command, the stop, the
/// counts and one usage entry per completion exactly (larry#93, larry#217).
#[test]
fn sim_a_session_seals_what_it_ran_and_spent() -> TestResult {
    let replies = vec![
        Reply::Spent {
            content: String::from("{\"command\":\"true\"}"),
            prompt: 11,
            completion: 5,
        },
        Reply::Spent {
            content: String::from("{\"done\":\"all good\"}"),
            prompt: 23,
            completion: 2,
        },
    ];
    let ran = session(
        "sim-run-spent",
        replies,
        Scripted::new(vec![Approval::Run]),
        HOST_POLICY,
    )?;
    assert_eq!(ran.requests.len(), 2);
    assert!(
        matches!(ran.ended.stop, Stop::EndTurn),
        "{:?}",
        ran.ended.stop
    );
    assert_eq!(member(&ran, "/results/0/exit_code")?, &json!(0));
    assert_eq!(member(&ran, "/session/stop")?, &json!("end_turn"));
    assert_eq!(member(&ran, "/session/commands")?, &json!(1));
    assert_eq!(member(&ran, "/session/declined")?, &json!(0));
    assert_eq!(member(&ran, "/session/handoff")?, &json!("none"));
    assert_eq!(
        member(&ran, "/session/isolation")?,
        &json!({"mode": "host"})
    );
    assert_eq!(member(&ran, "/denied")?, &json!([]));
    assert_eq!(
        member(&ran, "/authored")?,
        &json!([
            {"prompt_tokens": 11, "completion_tokens": 5,
             "cached_tokens": null, "cache_write_tokens": null},
            {"prompt_tokens": 23, "completion_tokens": 2,
             "cached_tokens": null, "cache_write_tokens": null},
        ])
    );
    assert_eq!(ran.observer.messages, vec![String::from("all good")]);
    assert!(
        !ran.ended.report.accepted(),
        "a session carries no diff, so it never verifies accepted"
    );
    Ok(())
}

/// A session stopped by the gateway, by a cancel before the first call or by a
/// cancel mid command seals that stop and what ran (larry#242).
#[test]
fn sim_a_stopped_session_seals_why_it_stopped() -> TestResult {
    let refused = session(
        "sim-run-refused",
        vec![Reply::Refused {
            status: 400,
            body: String::from("{\"error\":{\"message\":\"scripted\"}}"),
        }],
        Scripted::new(Vec::new()),
        HOST_POLICY,
    )?;
    assert!(
        matches!(refused.ended.stop, Stop::Refusal(ref line) if line.starts_with("model: ")),
        "{:?}",
        refused.ended.stop
    );
    assert_eq!(member(&refused, "/session/stop")?, &json!("refusal"));
    assert_eq!(member(&refused, "/session/commands")?, &json!(0));

    let early = session(
        "sim-run-early",
        Vec::new(),
        Scripted::cancelling(Vec::new(), Cancel::AfterAsks(0)),
        HOST_POLICY,
    )?;
    assert!(early.requests.is_empty(), "the model is never dialled");
    assert!(matches!(early.ended.stop, Stop::Cancelled));
    assert_eq!(member(&early, "/session/stop")?, &json!("cancelled"));

    let mid = session(
        "sim-run-mid",
        vec![Reply::from("{\"command\":\"sleep 30\"}")],
        Scripted::cancelling(vec![Approval::Run], Cancel::OnStart),
        HOST_POLICY,
    )?;
    assert!(matches!(mid.ended.stop, Stop::Cancelled));
    assert_eq!(member(&mid, "/session/stop")?, &json!("cancelled"));
    assert_eq!(member(&mid, "/session/interrupted")?, &json!(1));
    assert_eq!(member(&mid, "/results/0/interrupted")?, &json!(true));
    Ok(())
}

/// A session the gate denies runs nothing, dials nothing and says to the person
/// the very line its report seals (larry#164).
#[test]
fn sim_a_denied_session_says_the_line_it_sealed() -> TestResult {
    let ran = session("sim-run-denied", Vec::new(), Scripted::new(Vec::new()), "")?;
    assert!(ran.requests.is_empty(), "a denial dials nothing");
    let sealed = member(&ran, "/denied/0")?
        .as_str()
        .ok_or("the denial is not text")?
        .to_owned();
    assert!(sealed.starts_with("gate: "), "{sealed}");
    assert!(
        matches!(ran.ended.stop, Stop::Refusal(ref line) if *line == sealed),
        "{:?}",
        ran.ended.stop
    );
    assert_eq!(ran.observer.messages, vec![sealed]);
    assert!(!ran.ended.report.accepted());
    Ok(())
}

/// A session refused before anything runs names its exact typed error and the
/// model is never dialled: a blank, oversized or NUL instruction, a working
/// directory that is missing or a file, a confinement no host enforces, and an
/// evaluator door with no directory or outside it. An instruction of exactly the
/// bound is admitted (CWE-770).
#[test]
fn sim_a_refused_session_names_its_cause_before_dialling() -> TestResult {
    let state = scratch("sim-run-early-refusals")?;
    let work = scratch("sim-run-early-refusals")?;
    let file = work.join("marker");
    std::fs::write(&file, "marker")?;
    let at_bound = "x".repeat(MAX_INSTRUCTION_BYTES);
    let past_bound = "x".repeat(MAX_INSTRUCTION_BYTES.saturating_add(1));
    assert_eq!(check(&at_bound, &work), Ok(()), "the bound is admitted");
    assert_eq!(
        check(&past_bound, &work),
        Err(SessionError::InstructionTooLarge {
            len: MAX_INSTRUCTION_BYTES.saturating_add(1)
        })
    );
    let cases: [(&str, &Path, SessionError); 5] = [
        ("   \n ", &work, SessionError::EmptyInstruction),
        (
            &past_bound,
            &work,
            SessionError::InstructionTooLarge {
                len: MAX_INSTRUCTION_BYTES.saturating_add(1),
            },
        ),
        ("list\0files", &work, SessionError::NulInInstruction),
        ("read this", &file, SessionError::NotADirectory),
        (
            "read this",
            &work.join("missing"),
            SessionError::NotADirectory,
        ),
    ];
    let gateway = StubGateway::bind()?;
    let settings = settings(&state, gateway.url(), HOST_POLICY)?;
    let mut observer = Scripted::new(Vec::new());
    let ((), requests) = gateway.serve_while(Vec::new(), || {
        let mut run = |instruction: &str, cwd: &Path, isolation, door| {
            run_session(instruction, cwd, &settings, isolation, &mut observer, door)
        };
        for (instruction, cwd, want) in cases {
            let got = run(instruction, cwd, Isolation::Host, SessionDoor::Tenant);
            assert!(
                matches!(got, Err(ExecError::Session(ref refused)) if *refused == want),
                "{want}: {got:?}"
            );
        }
        let unconfined = run(
            "read this",
            &work,
            Isolation::Workspace(Platform::Other),
            SessionDoor::Tenant,
        );
        assert!(
            matches!(unconfined, Err(ExecError::Confine(_))),
            "{unconfined:?}"
        );
        let no_eval = run("read this", &work, Isolation::Host, SessionDoor::Eval);
        assert!(
            matches!(no_eval, Err(ExecError::StateDir(_))),
            "{no_eval:?}"
        );
        std::fs::create_dir_all(state.join("eval"))?;
        let outside = run("read this", &work, Isolation::Host, SessionDoor::Eval);
        assert!(
            matches!(outside, Err(ExecError::RepoInsideState { ref repo }) if *repo == *work),
            "{outside:?}"
        );
        Ok(())
    })?;
    assert!(requests.is_empty(), "no refusal dials the model");
    Ok(())
}

/// The sealed usage keeps one entry per completion, `null` where the gateway
/// reported none and every count where it did (larry#217).
#[test]
fn sim_authored_usage_keeps_every_completion() {
    let usage = [
        None,
        Some(Usage {
            prompt_tokens: 11,
            completion_tokens: 5,
            cached_tokens: Some(2),
            cache_write_tokens: Some(7),
        }),
    ];
    assert_eq!(
        authored_usage(&usage),
        vec![
            json!({"prompt_tokens": null, "completion_tokens": null,
                   "cached_tokens": null, "cache_write_tokens": null}),
            json!({"prompt_tokens": 11, "completion_tokens": 5,
                   "cached_tokens": 2, "cache_write_tokens": 7}),
        ]
    );
}

/// An observer that only answers approvals takes every default: it discards the
/// change it is asked to review, streams nowhere and never cancels (larry#243:
/// nothing reaches the folder when nobody can be asked).
#[test]
fn sim_an_observer_nobody_can_ask_discards_and_never_cancels() {
    struct Quiet;
    impl Observer for Quiet {
        fn approve(&mut self, _step: usize, _command: &str) -> Approval {
            Approval::Decline
        }
    }
    let mut observer = Quiet;
    assert_eq!(
        observer.review(&[String::from("1 file changed")]),
        Handoff::Discard
    );
    assert!(observer.stream_callback().is_none());
    assert!(!observer.cancelled());
}

/// A tenant's route template leads the system turn of every request and the
/// report names the route, so a session speaks its route's words (larry#205).
#[test]
fn sim_a_route_template_leads_the_system_turn() -> TestResult {
    let state = scratch("sim-run-template")?;
    let work = scratch("sim-run-template")?;
    let gateway = StubGateway::bind()?;
    let url = gateway.url().to_owned();
    let tenant = settings(&state, &url, HOST_POLICY)?
        .tenant_text()
        .to_owned();
    let key_env = format!("{}SIM", tenant_namespace(&tenant));
    let template = "Answer for the template route.";
    Ledger::open(&state)?.put_route(
        &tenant,
        &Route::new(&Draft {
            id: "templated",
            kind: Kind::OpenAiChat,
            base_url: &url,
            model: "sim-templated",
            key_env: &key_env,
            temperature: None,
            request_bytes: None,
            residency: None,
            prompt: Some(template),
            describe: "a templated route",
        })?,
        true,
    )?;
    let state_text = state
        .to_str()
        .ok_or("the state directory is not text")?
        .to_owned();
    let settings = Settings::from_env(move |name| match name {
        "LARRY_STATE_DIR" => Some(state_text.clone()),
        "PATH" => Some(String::from("/usr/bin:/bin")),
        "LARRY_POLICY" => Some(String::from(HOST_POLICY)),
        "LARRY_MODEL_ATTEMPTS" => Some(String::from("1")),
        _ => (name == key_env).then(|| String::from("sim-templated-key")),
    })?;
    let mut observer = Scripted::new(Vec::new());
    let (ended, requests) = gateway.serve_while(vec![Reply::from("{\"done\":\"ok\"}")], || {
        Ok(run_session(
            "use the template",
            &work,
            &settings,
            Isolation::Host,
            &mut observer,
            SessionDoor::Tenant,
        )?)
    })?;
    assert!(matches!(ended.stop, Stop::EndTurn), "{:?}", ended.stop);
    let request = requests.first().ok_or("one request")?;
    assert_eq!(request.at("/model")?, &json!("sim-templated"));
    assert_eq!(
        request.at("/messages/0/content")?,
        &json!(format!("{template}\n{SYSTEM}"))
    );
    assert_eq!(
        ended.report.json().at("/session/route/id")?,
        &json!("templated")
    );
    Ok(())
}

/// A session whose model never says done is stopped at 256 steps and seals that
/// stop and every decline (CWE-770).
#[test]
fn sim_a_session_past_its_step_bound_seals_that_stop() -> TestResult {
    let ran = session(
        "sim-run-bound",
        vec![Reply::from("{\"command\":\"true\"}"); 256],
        Scripted::new(Vec::new()),
        HOST_POLICY,
    )?;
    assert_eq!(ran.requests.len(), 256);
    assert!(
        matches!(ran.ended.stop, Stop::MaxTurnRequests),
        "{:?}",
        ran.ended.stop
    );
    assert_eq!(member(&ran, "/session/stop")?, &json!("max_turn_requests"));
    assert_eq!(member(&ran, "/session/declined")?, &json!(256));
    assert_eq!(member(&ran, "/session/commands")?, &json!(0));
    Ok(())
}

/// The settings' debug form names where they work and never a key, so a log of
/// them leaks nothing a variable held (CWE-532).
#[test]
fn sim_settings_debug_never_prints_a_key() -> TestResult {
    let state = scratch("sim-run-debug")?;
    let settings = settings(&state, "http://127.0.0.1:9/v1", "file.read")?;
    let shown = format!("{settings:?}");
    assert!(shown.starts_with("Settings { state_dir: "), "{shown}");
    assert!(shown.ends_with(", .. }"), "the rest is elided: {shown}");
    assert!(shown.contains("policy: {FileRead}"), "{shown}");
    assert!(!shown.contains(SIM_KEY), "the key is never shown: {shown}");
    Ok(())
}
