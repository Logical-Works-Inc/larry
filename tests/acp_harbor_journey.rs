//! Journeys against the shipped `larry acp` binary as Harbor's generic `acp`
//! agent drives it on a Terminal-Bench 2.0 task (larry#93).
//!
//! Source: the Agent Client Protocol v1 (initialize, session/new, session/prompt,
//! session/update, session/cancel, stdio transport); Harbor's task layout
//! (`instruction.md`, `task.toml`, `tests/test.sh`, the verifier writing
//! `/logs/verifier/reward.txt`); and the executor's own kill -9 contract (larry#75).
//! Requirement proved: a Harbor task's instruction runs as a terminal session the
//! model drives one command at a time, the commands change the working directory,
//! the task's verifier rewards it, and the sealed report verifies; the default
//! policy refuses with nothing run; a session stops at its step bound, at three
//! refused replies, and at a cancel before its next command; a kill -9 leaves a
//! journal the next start recovers; two state directories never share a report or
//! a key. Every case runs the real process against the seeded mock model, so no
//! test makes a paid model call. The local verifier run stands in for Harbor's
//! container: the same files, the same reward contract, no Docker.

#[path = "support/http_wire.rs"]
mod http_wire;
#[path = "support/larry_child.rs"]
mod larry_child;
#[path = "support/mock_model.rs"]
mod mock_model;
#[path = "support/origin.rs"]
mod origin;

use std::error::Error;
use std::ffi::{OsStr, OsString};
use std::fs;
use std::io::{BufRead, BufReader, Read, Write};
use std::path::Path;
use std::process::{Child, ChildStderr, ChildStdin, ChildStdout, Command, Stdio};
use std::time::Instant;

use lgwks_std::json::{self, Value, json};
use mock_model::{Config, Fault, Handle};
use origin::{TempDir, fixture};

/// What every test here returns.
type TestResult<T = ()> = Result<T, Box<dyn Error>>;

/// Every scope a session needs, `model.command` among them: what an operator
/// sets inside a Harbor task container.
const SESSION_POLICY: &str = "file.read,file.write,process.spawn,model.invoke,model.command";

/// Timed runs of the Harbor task.
const SAMPLES: u64 = 16;

/// The task's verifier, as Harbor runs it after the agent: reward 1 when the
/// working directory holds `hello.txt` with exactly `hello`.
const TEST_SH: &str = "#!/bin/sh\nmkdir -p \"$LOGS/verifier\"\nif [ \"$(cat \"$WORKDIR/hello.txt\" 2>/dev/null)\" = hello ]; then echo 1 > \"$LOGS/verifier/reward.txt\"; else echo 0 > \"$LOGS/verifier/reward.txt\"; fi\n";

/// One `larry acp` process: its protocol pipes and its log.
struct Agent {
    /// The process.
    child: Child,
    /// The protocol stream in; `None` once closed.
    stdin: Option<ChildStdin>,
    /// The protocol stream out.
    out: BufReader<ChildStdout>,
    /// The agent's log.
    log: BufReader<ChildStderr>,
}

impl Agent {
    /// Starts `larry acp` with only PATH and `env` set.
    fn start(env: &[(&str, &OsStr)]) -> TestResult<Self> {
        let (mut child, stdin, out) = larry_child::start("acp", env, Stdio::piped())?;
        let log = BufReader::new(child.stderr.take().ok_or("stderr was not piped")?);
        Ok(Self {
            child,
            stdin: Some(stdin),
            out,
            log,
        })
    }

    /// Writes one message as its line.
    fn send(&mut self, message: &Value) -> TestResult {
        let stdin = self.stdin.as_mut().ok_or("stdin is closed")?;
        let mut line = message.to_string();
        line.push('\n');
        stdin.write_all(line.as_bytes())?;
        stdin.flush()?;
        Ok(())
    }

    /// Reads one message; end of output is an error.
    fn recv(&mut self) -> TestResult<Value> {
        let mut line = String::new();
        if self.out.read_line(&mut line)? == 0 {
            return Err("larry acp closed its output".into());
        }
        Ok(json::from_str(&line)?)
    }

    /// Reads until the response to `id`, returning the notifications before it.
    fn until(&mut self, id: &Value) -> TestResult<(Vec<Value>, Value)> {
        let mut notes = Vec::new();
        loop {
            let message = self.recv()?;
            if message.get("method").is_some() {
                notes.push(message);
            } else if message["id"] == *id {
                return Ok((notes, message));
            }
        }
    }

    /// Sends a request and reads to its response.
    fn call(&mut self, request: &Value) -> TestResult<(Vec<Value>, Value)> {
        self.send(request)?;
        self.until(&request["id"])
    }

    /// Initializes and opens one session in `cwd`, returning its id.
    fn open(&mut self, cwd: &Path) -> TestResult<String> {
        let (_, init) = self.call(&json!({"jsonrpc":"2.0","id":0,"method":"initialize",
            "params":{"protocolVersion":1,"clientCapabilities":{"terminal":false}}}))?;
        assert_eq!(init["result"]["protocolVersion"], 1, "version 1");
        let (_, new) = self.call(&json!({"jsonrpc":"2.0","id":1,"method":"session/new",
            "params":{"cwd":cwd.to_string_lossy(),"mcpServers":[]}}))?;
        let session = new["result"]["sessionId"]
            .as_str()
            .ok_or("session/new answers a session id")?;
        Ok(session.to_owned())
    }

    /// Reads log lines until one holds `text`.
    fn logged(&mut self, text: &str) -> TestResult {
        let mut line = String::new();
        loop {
            line.clear();
            if self.log.read_line(&mut line)? == 0 {
                return Err(format!("the log ended before {text:?}").into());
            }
            if line.contains(text) {
                return Ok(());
            }
        }
    }

    /// Closes stdin and returns the exit code.
    fn finish(mut self) -> TestResult<Option<i32>> {
        drop(self.stdin.take());
        Ok(self.child.wait()?.code())
    }
}

impl Drop for Agent {
    fn drop(&mut self) {
        // Best effort: a journey that already ended its agent leaves nothing to kill.
        self.child.kill().ok();
        self.child.wait().ok();
    }
}

/// A `session/prompt` request carrying `text`.
fn prompt(id: u64, session: &str, text: &str) -> Value {
    json!({"jsonrpc":"2.0","id":id,"method":"session/prompt",
        "params":{"sessionId":session,"prompt":[{"type":"text","text":text}]}})
}

/// The settings an operator exports: the state directory, the mock gateway and,
/// when given, the policy.
fn operator(state: &Path, mock: &Handle, policy: Option<&str>) -> Vec<(&'static str, OsString)> {
    let mut env = vec![
        ("LARRY_STATE_DIR", state.as_os_str().to_owned()),
        (
            "LARRY_MODEL_URL",
            OsString::from(format!("http://{}/v1", mock.addr())),
        ),
        ("LARRY_MODEL_KEY", OsString::from("acp-key")),
        ("LARRY_MODEL_BACKOFF_MS", OsString::from("0")),
    ];
    if let Some(policy) = policy {
        env.push(("LARRY_POLICY", OsString::from(policy)));
    }
    env
}

/// Starts `larry acp` under `operator`'s settings.
fn agent(state: &Path, mock: &Handle, policy: Option<&str>) -> TestResult<Agent> {
    let env = operator(state, mock, policy);
    let pairs: Vec<(&str, &OsStr)> = env
        .iter()
        .map(|pair| (pair.0, pair.1.as_os_str()))
        .collect();
    Agent::start(&pairs)
}

/// A mock whose replies are `contents`, in order.
fn scripted(seed: u64, contents: &[&str]) -> Config {
    Config {
        contents: contents.iter().map(|text| (*text).to_owned()).collect(),
        ..Config::seeded(seed)
    }
}

/// The `sessionUpdate` kinds of `notes`, in order.
fn kinds(notes: &[Value]) -> Vec<String> {
    notes
        .iter()
        .filter_map(|note| note["params"]["update"]["sessionUpdate"].as_str())
        .map(str::to_owned)
        .collect()
}

/// The text of the `agent_message_chunk` among `notes`.
fn said(notes: &[Value]) -> Option<&str> {
    notes
        .iter()
        .find(|note| note["params"]["update"]["sessionUpdate"] == "agent_message_chunk")
        .and_then(|note| note["params"]["update"]["content"]["text"].as_str())
}

/// The text of message `index` of the model request `body`.
fn message_text(body: &[u8], index: usize) -> TestResult<String> {
    let request = json::from_slice::<Value>(body)?;
    let text = request["messages"][index]["content"]
        .as_str()
        .ok_or("the request carries that message")?;
    Ok(text.to_owned())
}

/// Runs `larry <subcommand>` to completion with only `env` set and `input` on
/// stdin; returns the exit code and stdout.
fn larry(
    subcommand: &str,
    env: &[(&str, &OsStr)],
    input: &[u8],
) -> TestResult<(Option<i32>, Vec<u8>)> {
    let (mut child, mut stdin, mut out) = larry_child::start(subcommand, env, Stdio::null())?;
    stdin.write_all(input)?;
    drop(stdin);
    let mut printed = Vec::new();
    out.read_to_end(&mut printed)?;
    Ok((child.wait()?.code(), printed))
}

/// Verifies `report` offline under the key `larry key` prints for `state`; returns
/// the exit code and the printed verdict.
fn verify(state: &Path, report: &Value) -> TestResult<(Option<i32>, Value)> {
    let (_, key) = larry("key", &[("LARRY_STATE_DIR", state.as_os_str())], b"")?;
    let trusted = OsString::from(String::from_utf8(key)?.trim_end());
    let (code, printed) = larry(
        "verify",
        &[("LARRY_RECEIPT_KEY", trusted.as_os_str())],
        report.to_string().as_bytes(),
    )?;
    let verdict = match json::from_slice::<Value>(&printed) {
        Ok(verdict) => verdict,
        Err(_) => Value::Null,
    };
    Ok((code, verdict))
}

/// Writes a Harbor task directory under `root` and returns its instruction.
fn harbor_task(root: &Path) -> TestResult<String> {
    let instruction = "Create a file named hello.txt in the working directory containing exactly the word hello.\n";
    fs::create_dir_all(root.join("tests"))?;
    fs::create_dir_all(root.join("environment"))?;
    fs::write(root.join("instruction.md"), instruction)?;
    fs::write(
        root.join("task.toml"),
        "version = \"1.0\"\n\n[verifier]\ntimeout_sec = 60.0\n\n[agent]\ntimeout_sec = 600.0\n",
    )?;
    fs::write(
        root.join("environment").join("Dockerfile"),
        "FROM ubuntu:24.04\nWORKDIR /app\n",
    )?;
    fs::write(root.join("tests").join("test.sh"), TEST_SH)?;
    Ok(fs::read_to_string(root.join("instruction.md"))?)
}

/// Runs the task's verifier against `work` and returns the reward it wrote.
fn reward(task: &Path, work: &Path, logs: &Path) -> TestResult<String> {
    let status = Command::new("sh")
        .arg(task.join("tests").join("test.sh"))
        .env_clear()
        .env("PATH", std::env::var("PATH")?)
        .env("WORKDIR", work)
        .env("LOGS", logs)
        .status()?;
    assert!(status.success(), "the verifier runs");
    let written = fs::read_to_string(logs.join("verifier").join("reward.txt"))?;
    Ok(written.trim().to_owned())
}

/// The sample at rank `per_cent` of the sorted microsecond samples.
fn rank(sorted: &[u128], per_cent: usize) -> TestResult<u128> {
    let at = sorted.len().saturating_mul(per_cent).div_ceil(100);
    Ok(*sorted.get(at.saturating_sub(1)).ok_or("no samples")?)
}

/// The Harbor journey: the instruction runs as a session, the model's command
/// writes the file, the verifier rewards it, and the sealed report verifies.
#[test]
fn a_harbor_task_runs_through_larry_acp_and_its_verifier_rewards_it() -> TestResult {
    let mut micros = Vec::new();
    for seed in 0..SAMPLES {
        let (state, task) = fixture()?;
        let work = TempDir::new()?;
        let logs = TempDir::new()?;
        let instruction = harbor_task(task.path())?;
        let replies = [
            r#"{"command":"printf 'hello\n' > hello.txt"}"#,
            "```json\n{\"done\":\"wrote hello.txt\"}\n```",
        ];
        mock_model::run(scripted(seed, &replies), |mock| -> TestResult {
            let started = Instant::now();
            let mut acp = agent(state.path(), mock, Some(SESSION_POLICY))?;
            let session = acp.open(work.path())?;
            let (notes, reply) = acp.call(&prompt(2, &session, &instruction))?;
            micros.push(started.elapsed().as_micros());
            assert_eq!(reply["result"]["stopReason"], "end_turn", "{reply}");
            assert_eq!(
                kinds(&notes),
                ["tool_call", "tool_call_update", "agent_message_chunk"],
                "one command, its result, the summary"
            );
            let call = &notes[0]["params"];
            assert_eq!(call["sessionId"], session.as_str(), "the session's update");
            assert_eq!(call["update"]["kind"], "execute", "an execute call");
            assert_eq!(
                notes[1]["params"]["update"]["status"], "completed",
                "exit 0"
            );
            assert_eq!(said(&notes), Some("wrote hello.txt"), "the model's summary");
            assert_eq!(mock.served(), 2, "two model calls");
            let asked = message_text(&mock.requests()[0], 1)?;
            assert!(
                asked.contains(instruction.trim()),
                "the instruction reaches the model"
            );
            assert_eq!(
                reward(task.path(), work.path(), logs.path())?,
                "1",
                "rewarded"
            );
            let report = &reply["result"]["_meta"]["larry"]["report"];
            assert_eq!(report["session"]["stop"], "end_turn", "the report's stop");
            assert_eq!(report["results"][0]["exit_code"], 0, "the command's record");
            let (code, verdict) = verify(state.path(), report)?;
            // A session has no diff for Larry to accept: the seal verifies, and the
            // task's own verifier decides the reward.
            assert_eq!(code, Some(2), "a verified, unaccepted report: {verdict}");
            assert_eq!(
                verdict["task_id"], report["task_id"],
                "the verdict names the run"
            );
            assert_eq!(acp.finish()?, Some(0), "end of input is a clean exit");
            Ok(())
        })?;
    }
    micros.sort_unstable();
    writeln!(
        std::io::stdout().lock(),
        "larry acp harbor session ({SAMPLES} samples, spawn to stopReason): p50 {} us p95 {} us p99 {} us",
        rank(&micros, 50)?,
        rank(&micros, 95)?,
        rank(&micros, 99)?
    )?;
    Ok(())
}

/// The default policy holds no `model.command`: the prompt is a refusal naming it,
/// the model is never called and nothing is written.
#[test]
fn the_default_policy_refuses_a_session_and_runs_nothing() -> TestResult {
    let (state, work) = fixture()?;
    let replies = [r#"{"command":"touch ran.txt"}"#];
    mock_model::run(scripted(7, &replies), |mock| -> TestResult {
        let mut acp = agent(state.path(), mock, None)?;
        let session = acp.open(work.path())?;
        let (notes, reply) = acp.call(&prompt(2, &session, "touch ran.txt"))?;
        assert_eq!(reply["result"]["stopReason"], "refusal", "{reply}");
        let line = "gate: policy_refuses: model.command";
        assert_eq!(said(&notes), Some(line), "the refusal names the scope");
        assert_eq!(mock.served(), 0, "no model call");
        assert!(!work.path().join("ran.txt").exists(), "nothing ran");
        let report = &reply["result"]["_meta"]["larry"]["report"];
        assert_eq!(report["denied"][0], line, "the denial is sealed");
        assert_eq!(
            verify(state.path(), report)?.0,
            Some(2),
            "its seal verifies"
        );
        Ok(())
    })
}

/// 48 commands is the bound: the session ends `max_turn_requests` with 48 records.
#[test]
fn a_session_past_its_step_bound_ends_max_turn_requests() -> TestResult {
    let (state, work) = fixture()?;
    let replies = [r#"{"command":"true"}"#; 49];
    mock_model::run(scripted(11, &replies), |mock| -> TestResult {
        let mut acp = agent(state.path(), mock, Some(SESSION_POLICY))?;
        let session = acp.open(work.path())?;
        let (notes, reply) = acp.call(&prompt(2, &session, "loop"))?;
        assert_eq!(
            reply["result"]["stopReason"], "max_turn_requests",
            "{reply}"
        );
        assert_eq!(
            mock.served(),
            48,
            "one model call per command, none past the bound"
        );
        assert_eq!(notes.len(), 96, "a call and its result per command");
        let report = &reply["result"]["_meta"]["larry"]["report"];
        assert_eq!(report["session"]["commands"], 48, "48 records");
        Ok(())
    })
}

/// Three refused replies in a row end the session as a refusal; nothing runs.
#[test]
fn three_refused_replies_in_a_row_end_the_session() -> TestResult {
    let (state, work) = fixture()?;
    let replies = ["run ls", "[1]", r#"{"command":"ls","done":"x"}"#];
    mock_model::run(scripted(13, &replies), |mock| -> TestResult {
        let mut acp = agent(state.path(), mock, Some(SESSION_POLICY))?;
        let session = acp.open(work.path())?;
        let (notes, reply) = acp.call(&prompt(2, &session, "list"))?;
        assert_eq!(reply["result"]["stopReason"], "refusal", "{reply}");
        assert_eq!(
            said(&notes),
            Some(
                "model: 3 replies in a row refused: the object must hold exactly one key, command or done"
            ),
            "the last refusal is named"
        );
        assert_eq!(mock.served(), 3, "three model calls");
        let told = message_text(&mock.requests()[1], 3)?;
        assert!(
            told.starts_with("Your reply was refused: the reply is not JSON"),
            "the model is told why: {told}"
        );
        Ok(())
    })
}

/// A cancel during a command stops the session before its next model call.
#[test]
fn a_cancel_stops_the_session_before_its_next_command() -> TestResult {
    let (state, work) = fixture()?;
    let fifo = work.path().join("gate.fifo");
    assert!(
        Command::new("mkfifo").arg(&fifo).status()?.success(),
        "mkfifo"
    );
    let replies = [
        r#"{"command":"cat gate.fifo"}"#,
        r#"{"command":"touch after.txt"}"#,
    ];
    mock_model::run(scripted(17, &replies), |mock| -> TestResult {
        let mut acp = agent(state.path(), mock, Some(SESSION_POLICY))?;
        let session = acp.open(work.path())?;
        acp.send(&prompt(2, &session, "wait"))?;
        let started = acp.recv()?;
        assert_eq!(
            started["params"]["update"]["sessionUpdate"], "tool_call",
            "running"
        );
        acp.send(&json!({"jsonrpc":"2.0","method":"session/cancel",
            "params":{"sessionId":session}}))?;
        acp.logged("cancel requested")?;
        // The writer's open waits for cat's; the write lets cat end.
        fs::OpenOptions::new()
            .write(true)
            .open(&fifo)?
            .write_all(b"go\n")?;
        let (notes, reply) = acp.until(&json!(2))?;
        assert_eq!(reply["result"]["stopReason"], "cancelled", "{reply}");
        assert_eq!(
            kinds(&notes),
            ["tool_call_update"],
            "the running command ends"
        );
        assert_eq!(mock.served(), 1, "no model call after the cancel");
        assert!(
            !work.path().join("after.txt").exists(),
            "no command after the cancel"
        );
        Ok(())
    })
}

/// A kill -9 mid-command leaves the session's journal; the next start recovers
/// it before its own session runs.
#[test]
fn a_kill_9_mid_session_leaves_a_journal_the_next_start_recovers() -> TestResult {
    let (state, work) = fixture()?;
    let fifo = work.path().join("gate.fifo");
    assert!(
        Command::new("mkfifo").arg(&fifo).status()?.success(),
        "mkfifo"
    );
    let replies = [r#"{"command":"cat gate.fifo"}"#, r#"{"done":"recovered"}"#];
    mock_model::run(scripted(19, &replies), |mock| -> TestResult {
        let mut killed = agent(state.path(), mock, Some(SESSION_POLICY))?;
        let session = killed.open(work.path())?;
        killed.send(&prompt(2, &session, "wait"))?;
        let started = killed.recv()?;
        assert_eq!(
            started["params"]["update"]["sessionUpdate"], "tool_call",
            "running"
        );
        killed.child.kill()?;
        killed.child.wait()?;
        let mut next = agent(state.path(), mock, Some(SESSION_POLICY))?;
        let session = next.open(work.path())?;
        let (_, reply) = next.call(&prompt(2, &session, "again"))?;
        assert_eq!(reply["result"]["stopReason"], "end_turn", "{reply}");
        let recovered = fs::read_dir(state.path().join("journals"))?
            .filter_map(Result::ok)
            .filter(|entry| entry.path().extension() == Some(OsStr::new("recovered")))
            .count();
        assert_eq!(recovered, 1, "the killed session's journal was recovered");
        // Opening read-write never blocks; the byte frees a cat the reap missed.
        fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(&fifo)?
            .write_all(b"x")?;
        Ok(())
    })
}

/// One tenant's session: touches its own file and returns its report and the
/// requests its gateway served.
fn tenant(index: usize, state: &Path, work: &Path) -> TestResult<(Value, u64)> {
    let name = format!("tenant{index}.txt");
    let command = format!(r#"{{"command":"touch {name}"}}"#);
    let config = scripted(23, &[command.as_str(), r#"{"done":"ok"}"#]);
    mock_model::run(config, |mock| {
        let mut acp = agent(state, mock, Some(SESSION_POLICY))?;
        let session = acp.open(work)?;
        let (_, reply) = acp.call(&prompt(2, &session, "touch"))?;
        assert!(work.join(&name).exists(), "its own file");
        let report = reply["result"]["_meta"]["larry"]["report"].clone();
        Ok((report, mock.served()))
    })
}

/// Two state directories run sessions at once: each report verifies only under
/// its own key and each gateway sees only its own session.
#[test]
fn two_state_directories_run_sessions_at_once_and_keep_their_own_reports() -> TestResult {
    let (first_state, first_work) = fixture()?;
    let (second_state, second_work) = fixture()?;
    let (one, two) = std::thread::scope(|scope| {
        // A boxed error cannot cross threads; each tenant's failure comes back as text.
        let one = scope.spawn(|| {
            tenant(0, first_state.path(), first_work.path()).map_err(|cause| cause.to_string())
        });
        let two = scope.spawn(|| {
            tenant(1, second_state.path(), second_work.path()).map_err(|cause| cause.to_string())
        });
        (one.join(), two.join())
    });
    let ((first, first_served), (second, second_served)) = match (one, two) {
        (Ok(one), Ok(two)) => (one?, two?),
        _ => return Err("a tenant thread panicked".into()),
    };
    assert_eq!(
        (first_served, second_served),
        (2, 2),
        "each gateway saw its own session"
    );
    assert_ne!(first["task_id"], second["task_id"], "distinct runs");
    assert_eq!(
        verify(first_state.path(), &first)?.0,
        Some(2),
        "first under its key"
    );
    assert_eq!(
        verify(second_state.path(), &second)?.0,
        Some(2),
        "second under its key"
    );
    assert_eq!(
        verify(first_state.path(), &second)?.0,
        Some(1),
        "never under the other's"
    );
    Ok(())
}

/// One session in `work` whose first reply comes padded the way OpenRouter sends
/// it through 9router (larry#128); returns the gateway's trace hash and the key it saw.
fn padded_session(state: &Path, work: &Path) -> TestResult<(String, Option<String>)> {
    let config = Config {
        script: vec![Some(Fault::PaddedDone)],
        ..scripted(
            31,
            &[r#"{"command":"touch padded.txt"}"#, r#"{"done":"ok"}"#],
        )
    };
    mock_model::run(config, |mock| {
        let mut acp = agent(state, mock, Some(SESSION_POLICY))?;
        let session = acp.open(work)?;
        let (_, reply) = acp.call(&prompt(2, &session, "pad"))?;
        assert_eq!(reply["result"]["stopReason"], "end_turn", "{reply}");
        assert!(work.join("padded.txt").exists(), "the padded reply ran");
        let key = mock.authorizations().first().cloned().flatten();
        Ok((mock.trace_hash(), key))
    })
}

/// A padded reply still runs, the operator's key reaches the gateway, and the same
/// seed in the same directory gives the same gateway trace.
#[test]
fn a_padded_reply_runs_and_the_same_seed_replays_the_same_trace() -> TestResult {
    let (state, work) = fixture()?;
    let (first, key) = padded_session(state.path(), work.path())?;
    fs::remove_file(work.path().join("padded.txt"))?;
    let (second, _) = padded_session(state.path(), work.path())?;
    assert_eq!(key.as_deref(), Some("Bearer acp-key"), "the operator's key");
    assert_eq!(first, second, "same seed, same requests, same trace");
    Ok(())
}

/// A gateway refusal carrying escape sequences ends the session as a refusal whose
/// message carries none of them, and nothing runs.
#[test]
fn a_hostile_gateway_refusal_ends_the_session_with_a_clean_message() -> TestResult {
    let (state, work) = fixture()?;
    let config = Config {
        script: vec![Some(Fault::HostileRefusal { status: 401 })],
        ..scripted(37, &[r#"{"command":"touch ran.txt"}"#])
    };
    mock_model::run(config, |mock| -> TestResult {
        let mut acp = agent(state.path(), mock, Some(SESSION_POLICY))?;
        let session = acp.open(work.path())?;
        let (notes, reply) = acp.call(&prompt(2, &session, "go"))?;
        assert_eq!(reply["result"]["stopReason"], "refusal", "{reply}");
        let message = said(&notes).ok_or("the refusal is said")?;
        assert!(
            message.starts_with("model: "),
            "the gateway's refusal: {message}"
        );
        assert!(
            !message.contains('\u{1b}') && !message.contains('\u{7}'),
            "no escape sequence reaches the client"
        );
        assert!(!work.path().join("ran.txt").exists(), "nothing ran");
        Ok(())
    })
}

/// A cancel sent while the model is still answering ends the prompt `cancelled`
/// and its reply never runs.
#[test]
fn a_cancel_while_the_model_answers_runs_no_command() -> TestResult {
    let (state, work) = fixture()?;
    let config = Config {
        script: vec![Some(Fault::Delay)],
        ..scripted(41, &[r#"{"command":"touch ran.txt"}"#])
    };
    mock_model::run(config, |mock| -> TestResult {
        let mut acp = agent(state.path(), mock, Some(SESSION_POLICY))?;
        let session = acp.open(work.path())?;
        acp.send(&prompt(2, &session, "go"))?;
        acp.send(&json!({"jsonrpc":"2.0","method":"session/cancel",
            "params":{"sessionId":session}}))?;
        acp.logged("cancel requested")?;
        mock.release_delayed();
        let (notes, reply) = acp.until(&json!(2))?;
        assert_eq!(reply["result"]["stopReason"], "cancelled", "{reply}");
        assert!(notes.is_empty(), "no command started: {notes:?}");
        assert!(mock.served() <= 1, "at most the call already made");
        assert!(!work.path().join("ran.txt").exists(), "nothing ran");
        Ok(())
    })
}

/// Through the binary: a request before initialize and a relative cwd are refused,
/// and the agent keeps serving.
#[test]
fn a_request_before_initialize_and_a_relative_cwd_are_refused() -> TestResult {
    let (state, _work) = fixture()?;
    mock_model::run(Config::seeded(29), |mock| -> TestResult {
        let mut acp = agent(state.path(), mock, Some(SESSION_POLICY))?;
        let (_, early) = acp.call(&json!({"jsonrpc":"2.0","id":5,"method":"session/new",
            "params":{"cwd":"/","mcpServers":[]}}))?;
        assert_eq!(
            early["error"]["message"], "initialize must come first",
            "order"
        );
        let (_, init) = acp.call(&json!({"jsonrpc":"2.0","id":6,"method":"initialize",
            "params":{"protocolVersion":1}}))?;
        assert_eq!(init["result"]["authMethods"], json!([]), "no auth");
        let (_, relative) = acp.call(&json!({"jsonrpc":"2.0","id":7,"method":"session/new",
            "params":{"cwd":"work","mcpServers":[]}}))?;
        assert_eq!(relative["error"]["code"], -32602, "relative cwd");
        assert_eq!(mock.served(), 0, "no model call");
        assert_eq!(acp.finish()?, Some(0), "clean exit");
        Ok(())
    })
}
