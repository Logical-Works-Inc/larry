use super::*;

/// larry#123: one task over MCP against the mock model, its receipt verified,
/// for 8 seeds, each replayed to the same model trace and the same diff.
#[test]
fn a_submitted_task_runs_against_the_mock_model_and_its_receipt_verifies() -> TestResult<()> {
    let mut samples: Vec<Duration> = Vec::new();
    for seed in 0..TASK_SEEDS {
        let state = origin::TempDir::new()?;
        let first = accepted_run(seed, state.path())?;
        let again = accepted_run(seed, state.path())?;
        assert_eq!(
            first.trace, again.trace,
            "seed {seed}: one seed, one model trace"
        );
        assert_eq!(
            first.diff, again.diff,
            "seed {seed}: one seed, one sealed diff"
        );
        samples.push(first.took);
        samples.push(again.took);
    }
    samples.sort_unstable();
    let micros: Vec<u128> = samples.iter().map(Duration::as_micros).collect();
    let line = format!(
        "mcp submitted task (clone, 2 completions, 2 commands, seal): n={} p50={}us p95={}us p99={}us\n",
        micros.len(),
        percentile(&micros, 50)?,
        percentile(&micros, 95)?,
        percentile(&micros, 99)?
    );
    std::io::stderr().lock().write_all(line.as_bytes())?;
    Ok(())
}

/// larry#123, forge#158: a rejected run, every model fault, a symlink escape, a
/// missing model key and a bad repository are failed tool results, never success.
#[test]
fn a_rejected_or_refused_task_is_a_failed_tool_result_never_success() -> TestResult<()> {
    let (state, origin) = origin::fixture()?;
    let files = ["README"];
    let rejected = mock_model::run(Config::seeded(20), |handle: &Handle| -> TestResult<Value> {
        let mut session = connected(&operator(state.path(), handle.addr(), &[]))?;
        let result = submit(&mut session, &briefed(origin.path(), &files, &["exit 3"]))?;
        assert_eq!(session.finish()?.code(), Some(0), "clean exit");
        Ok(result)
    })?;
    let (code, _) = failure_of(&rejected)?;
    assert_eq!(code, "task_rejected", "a failing command is a rejected run");
    let report = rejected.at("/structuredContent/report")?;
    assert_eq!(report["accepted"], false, "sealed as rejected");
    assert_eq!(
        verify_offline(state.path(), report)?.0,
        Some(2),
        "its receipt verifies as rejected"
    );

    let task = briefed(origin.path(), &files, &["true"]);
    every_model_fault_refuses_the_task(&task)?;

    // A zero-wait retry goes ahead, and a padded OpenRouter reply still parses.
    let flaky = Config {
        script: vec![
            Some(Fault::ServerError { status: 503 }),
            Some(Fault::PaddedDone),
        ],
        ..Config::seeded(22)
    };
    let retried = origin::TempDir::new()?;
    mcp_task::provision(retried.path(), origin.path())?;
    mock_model::run(flaky, |handle: &Handle| -> TestResult<()> {
        let mut session = connected(&operator(retried.path(), handle.addr(), &[]))?;
        let result = submit(&mut session, &task)?;
        assert_eq!(
            *result.at("/isError")?,
            false,
            "a 503 then a padded reply is accepted: {result}"
        );
        assert_eq!(handle.served(), 2, "the 503 was retried once");
        assert_eq!(session.finish()?.code(), Some(0), "clean exit");
        Ok(())
    })?;

    // A symlink the origin carries is never written through.
    let outside = origin::TempDir::new()?;
    std::os::unix::fs::symlink(outside.path(), origin.path().join("escape"))?;
    origin::commit_all(origin.path(), "add a symlink")?;
    let escape = briefed(origin.path(), &["escape/x.rs"], &["true"]);
    let (code, message, served) = refused_with(&escape, Vec::new(), &[])?;
    assert_eq!(code, "task_refused", "a symlink escape is refused");
    assert!(
        message.contains("symlink"),
        "the planner sees why: {message}"
    );
    assert_eq!(served, 0, "the model is never asked");
    assert_eq!(
        fs::read_dir(outside.path())?.count(),
        0,
        "nothing was written outside the clone"
    );

    // No model key: the executor is not connected; a non-repository is refused.
    let keyless = origin::TempDir::new()?;
    mcp_task::provision(keyless.path(), origin.path())?;
    let mut session = connected(&[("LARRY_STATE_DIR", keyless.path().as_os_str().to_owned())])?;
    let (code, message) = failure_of(&submit(&mut session, &task)?)?;
    assert_eq!(code, "executor_not_connected", "no key, no executor");
    assert!(
        message.contains("LARRY_MODEL_KEY"),
        "the message names the variable: {message}"
    );
    assert_eq!(session.finish()?.code(), Some(0), "clean exit");
    let empty = origin::TempDir::new()?;
    let (code, message, served) =
        refused_with(&briefed(empty.path(), &files, &["true"]), Vec::new(), &[])?;
    assert_eq!(
        (code.as_str(), served),
        ("task_refused", 0),
        "a non-repository is refused before the model"
    );
    assert!(message.contains("could not clone"), "{message}");
    Ok(())
}
