//! Seeded controls of `larry-rerank` without the model: each seed draws candidate sets, faults,
//! bounds, receipt paths and engine scores, and the real parser, admission, ranking and receipt
//! writer run against them with a seeded stand-in scorer. They port the four candidate controls
//! of `tests/test_rerank_local.py` (larry#150) at seeded inputs instead of one fixed line, and
//! add the bounds, ranking and receipt rules the Python suite never exercised. Source: the Python
//! tool's messages, RFC 8259 (JSON text), POSIX `O_EXCL`. Requirement: every legal request ranks
//! like the reference and leaves a receipt `larry governance` admits; every illegal one is
//! refused with its exact message and writes nothing.

use std::ffi::OsString;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use lgwks_std::hash;
use lgwks_std::json::{self, Value};
use lgwks_std::seeded::Seeded;

use super::{Candidate, Deadline, Request, RerankError, Scorer, candidates, run};
use crate::governance::receipts::receipt_errors;

/// Failures propagate instead of panicking.
type Check<T = ()> = Result<T, Box<dyn std::error::Error>>;

/// Seeds each control runs.
const SEEDS: u64 = 64;

/// Distinct scratch roots made by this test binary.
static SCRATCHES: AtomicU64 = AtomicU64::new(0);

/// A scratch working directory, removed when dropped.
struct Root(PathBuf);

impl Root {
    /// A fresh empty directory.
    fn new() -> Check<Self> {
        let serial = SCRATCHES.fetch_add(1, Ordering::Relaxed);
        let path =
            std::env::temp_dir().join(format!("larry-rerank-{}-{serial}", std::process::id()));
        fs::create_dir_all(&path)?;
        Ok(Self(path))
    }
}

impl Drop for Root {
    fn drop(&mut self) {
        // Best effort: a leftover scratch directory never fails a control.
        drop(fs::remove_dir_all(&self.0));
    }
}

/// A seeded engine: fixed scores per call, counting calls, refusing on demand.
struct Stub {
    /// The scores it returns.
    scores: Vec<f64>,
    /// How many times it ran.
    calls: usize,
    /// When set, the engine times out instead of scoring.
    timeout: bool,
}

impl Scorer for Stub {
    fn score(
        &mut self,
        _query: &str,
        rows: &[Candidate],
        _batch: usize,
        _deadline: Deadline,
    ) -> Result<Vec<f64>, RerankError> {
        self.calls = self.calls.saturating_add(1);
        if self.timeout {
            return Err(RerankError::Timeout(7));
        }
        Ok(self.scores.iter().copied().take(rows.len()).collect())
    }
}

/// A seeded draw below `bound`.
fn draw(rng: &mut Seeded, bound: usize) -> Check<usize> {
    rng.index(bound)
        .map_err(|cause| format!("{cause:?}").into())
}

/// A seeded non-empty text mixing ASCII, escapes and multi-byte characters.
fn text(rng: &mut Seeded) -> Check<String> {
    let pieces = [
        "alpha",
        "β",
        "\"quoted\"",
        "line\nbreak",
        "tab\t",
        "😀",
        "\\",
        "日本",
    ];
    let mut out = String::new();
    for _ in 0..=draw(rng, 4)? {
        out.push_str(pieces.get(draw(rng, pieces.len())?).ok_or("piece")?);
    }
    Ok(out)
}

/// A seeded valid candidate set and its JSONL, with blank lines and extra keys mixed in.
fn valid_set(rng: &mut Seeded) -> Check<(Vec<(String, String)>, String)> {
    let count = draw(rng, 8)?.saturating_add(1);
    let mut rows = Vec::with_capacity(count);
    let mut lines = String::new();
    for index in 0..count {
        let id = format!("c{index}-{}", draw(rng, 1000)?);
        let body = text(rng)?;
        if draw(rng, 3)? == 0 {
            lines.push_str("  \n");
        }
        let mut object = json::json!({"id": id, "text": body});
        if draw(rng, 2)? == 0
            && let Some(map) = object.as_object_mut()
        {
            map.insert(String::from("extra"), Value::from(index));
        }
        lines.push_str(&json::to_string(&object)?);
        lines.push('\n');
        rows.push((id, body));
    }
    Ok((rows, lines))
}

/// A request for `root`'s receipt `name`.
fn request(phase: &str, name: &str, extra: &[(&str, String)]) -> Check<Request> {
    let mut args = vec![
        String::from("--phase"),
        String::from(phase),
        String::from("--query"),
        String::from("retry budget"),
        String::from("--receipt"),
        format!("tools/receipts/{name}"),
    ];
    for &(flag, ref value) in extra {
        args.push(String::from(flag));
        args.push(value.clone());
    }
    Ok(parse(args)?)
}

/// Ports `test_accepts_unique_nonempty_candidates`: any valid set parses in input order.
#[test]
fn sim_valid_candidate_sets_parse_in_input_order() -> Check {
    for seed in 0..SEEDS {
        let mut rng = Seeded::from_seed(seed);
        let (rows, lines) = valid_set(&mut rng)?;
        let parsed = candidates(lines.as_bytes())?;
        let got: Vec<(String, String)> = parsed.into_iter().map(|row| (row.id, row.text)).collect();
        assert_eq!(got, rows, "seed {seed}: the set parses in order");
    }
    Ok(())
}

/// One seeded malformed input and the message it must be refused with.
fn malformed(rng: &mut Seeded) -> Check<(Vec<u8>, &'static str)> {
    let good = r#"{"id":"a","text":"alpha"}"#;
    Ok(match draw(rng, 11)? {
        0 => (
            format!("{good}\n{{\"id\":\"a\",\"text\":\"beta\"}}\n").into_bytes(),
            "candidate ids must be unique",
        ),
        1 => (
            b"{\"id\":\"a\",\"text\":\"\"}\n".to_vec(),
            "each candidate needs non-empty string id and text",
        ),
        2 => (
            b"{\"text\":\"alpha\"}\n".to_vec(),
            "each candidate needs non-empty string id and text",
        ),
        3 => (
            b"{\"id\":7,\"text\":\"alpha\"}\n".to_vec(),
            "each candidate needs non-empty string id and text",
        ),
        4 => {
            let shapes = ["\"not an object\"", "7", "[1,2]", "null", "true"];
            let shape = shapes.get(draw(rng, shapes.len())?).ok_or("shape")?;
            (
                format!("{good}\n{shape}\n").into_bytes(),
                "each candidate must be a JSON object",
            )
        }
        5 => (
            format!(
                "{{\"id\":\"a\",\"text\":\"{}\"}}\n",
                "x".repeat(draw(rng, 64)?.saturating_add(256 * 1024 + 1))
            )
            .into_bytes(),
            "candidate text exceeds 262144 bytes",
        ),
        6 => (
            " ".repeat(draw(rng, 64)?.saturating_add(2 * 1024 * 1024 + 1))
                .into_bytes(),
            "candidate input exceeds 2097152 bytes",
        ),
        7 => {
            let lines: String = (0..257)
                .map(|index| format!("{{\"id\":\"c{index}\",\"text\":\"t\"}}\n"))
                .collect();
            (
                lines.into_bytes(),
                "candidate count must be between 1 and 256",
            )
        }
        8 => {
            let blanks = ["", "\n", "  \n\n", "\t\n"];
            let blank = blanks.get(draw(rng, blanks.len())?).ok_or("blank")?;
            (
                blank.as_bytes().to_vec(),
                "candidate count must be between 1 and 256",
            )
        }
        9 => (
            format!("{good}\n{{\"id\":\n").into_bytes(),
            "candidate line is not JSON",
        ),
        _ => {
            let mut bytes = format!("{good}\n").into_bytes();
            bytes.extend_from_slice(b"{\"id\":\"b\",\"text\":\"\xff\"}\n");
            (bytes, "candidate input is not UTF-8")
        }
    })
}

/// Ports `test_rejects_duplicate_ids`, `test_rejects_missing_text` and
/// `test_rejects_non_object_candidate`, and adds every other input bound: each seeded fault gets
/// its exact message.
#[test]
fn sim_each_malformed_candidate_input_gets_its_exact_error() -> Check {
    for seed in 0..SEEDS {
        let mut rng = Seeded::from_seed(seed);
        let (bytes, message) = malformed(&mut rng)?;
        let Err(error) = candidates(bytes.as_slice()) else {
            return Err(format!("seed {seed}: {message} was admitted").into());
        };
        assert!(
            error.to_string().starts_with(message),
            "seed {seed}: expected {message}, got {error}"
        );
    }
    Ok(())
}

/// The reference ranking: highest score first, ties in input order, `top_k` kept.
fn reference(rows: &[(String, String)], scores: &[f64], top_k: usize) -> Vec<(String, f64)> {
    let mut order: Vec<usize> = (0..rows.len()).collect();
    order.sort_by(|&left, &right| {
        let (Some(&first), Some(&second)) = (scores.get(left), scores.get(right)) else {
            return std::cmp::Ordering::Equal;
        };
        second.total_cmp(&first)
    });
    order
        .into_iter()
        .take(top_k)
        .filter_map(|index| {
            let (row, score) = (rows.get(index)?, scores.get(index)?);
            Some((row.0.clone(), *score))
        })
        .collect()
}

/// Every legal request ranks like the reference, writes a receipt the governance checker admits,
/// and refuses to overwrite it.
#[test]
fn sim_ranking_and_receipt_match_the_reference_for_every_seed() -> Check {
    for seed in 0..SEEDS {
        let mut rng = Seeded::from_seed(seed);
        let (rows, lines) = valid_set(&mut rng)?;
        let mut scores = Vec::with_capacity(rows.len());
        for _ in &rows {
            // Draws from a small set so ties are common and stability is exercised.
            scores.push(f64::from(u8::try_from(draw(&mut rng, 5)?)?) / 4.0);
        }
        let top_k = draw(&mut rng, rows.len())?.saturating_add(1);
        let root = Root::new()?;
        let mut receipts = Vec::new();
        for phase in ["pre-exec", "post-exec"] {
            let name = format!("{phase}-{seed}.json");
            let request = request(phase, &name, &[("--top-k", top_k.to_string())])?;
            let mut stub = Stub {
                scores: scores.clone(),
                calls: 0,
                timeout: false,
            };
            let line = run(&request, lines.as_bytes(), &root.0, &mut stub)?;
            let written = fs::read_to_string(root.0.join("tools/receipts").join(&name))?;
            assert_eq!(
                written,
                format!("{line}\n"),
                "seed {seed}: printed = written"
            );
            receipts.push((name, line));
        }
        let errors = receipt_errors(&root.0);
        assert!(
            errors.is_empty(),
            "seed {seed}: governance admits: {errors:?}"
        );
        let Some(first) = receipts.first() else {
            return Err("no receipt".into());
        };
        let (name, line) = (&first.0, &first.1);
        let receipt: Value = json::from_str(line)?;
        let got: Vec<(String, f64)> = receipt
            .get("results")
            .and_then(Value::as_array)
            .ok_or("results")?
            .iter()
            .filter_map(|result| {
                Some((
                    String::from(result.get("id")?.as_str()?),
                    result.get("score")?.as_f64()?,
                ))
            })
            .collect();
        assert_eq!(
            got,
            reference(&rows, &scores, top_k),
            "seed {seed}: ranking"
        );
        let query = hash::blake3(b"retry budget").to_hex();
        assert_eq!(
            receipt.get("query_blake3").and_then(Value::as_str),
            Some(query.as_str()),
            "seed {seed}: the query digest"
        );
        assert_eq!(
            receipt.get("candidate_count").and_then(Value::as_u64),
            u64::try_from(rows.len()).ok(),
            "seed {seed}: the count"
        );
        let request = request("pre-exec", name, &[("--top-k", top_k.to_string())])?;
        let mut stub = Stub {
            scores: scores.clone(),
            calls: 0,
            timeout: false,
        };
        let again = run(&request, lines.as_bytes(), &root.0, &mut stub);
        assert!(
            matches!(again, Err(RerankError::Io(ref cause)) if cause.kind() == io::ErrorKind::AlreadyExists),
            "seed {seed}: an existing receipt is never overwritten: {again:?}"
        );
        assert_eq!(
            fs::read_to_string(root.0.join("tools/receipts").join(name))?,
            format!("{line}\n"),
            "seed {seed}: the first receipt is intact"
        );
        mode_is_private(&root.0.join("tools/receipts").join(name), seed)?;
    }
    Ok(())
}

/// The receipt is readable and writable by its owner only.
fn mode_is_private(path: &Path, seed: u64) -> Check {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        let mode = fs::metadata(path)?.permissions().mode() & 0o777;
        assert_eq!(mode, 0o600, "seed {seed}: the receipt is owner-only");
    }
    #[cfg(not(unix))]
    {
        drop((path, seed));
    }
    Ok(())
}

/// An illegal request: its extra flags, its receipt path and its message.
type Illegal = (Vec<(&'static str, String)>, String, &'static str);

/// One seeded illegal request for a set of `count` candidates.
fn illegal(rng: &mut Seeded, count: usize) -> Check<Illegal> {
    let bounds = "top-k, batch-size and max-seconds must be bounded";
    let path = "receipt path must be directly under tools/receipts/";
    let fine = String::from("tools/receipts/ok.json");
    Ok(match draw(rng, 8)? {
        0 => (vec![("--top-k", String::from("0"))], fine, bounds),
        1 => (
            vec![("--top-k", count.saturating_add(1).to_string())],
            fine,
            bounds,
        ),
        2 => (vec![("--batch-size", String::from("0"))], fine, bounds),
        3 => (vec![("--batch-size", String::from("257"))], fine, bounds),
        4 => (vec![("--max-seconds", String::from("0"))], fine, bounds),
        5 => (vec![("--max-seconds", String::from("3601"))], fine, bounds),
        6 => {
            let paths = [
                "tools/receipts/nested/x.json",
                "tools/x.json",
                "x.json",
                "tools/receipts/../x.json",
            ];
            let chosen = paths.get(draw(rng, paths.len())?).ok_or("path")?;
            (Vec::new(), String::from(*chosen), path)
        }
        _ => (Vec::new(), String::from("/x.json"), path),
    })
}

/// Every out-of-range bound and every receipt path outside `tools/receipts/` is refused with its
/// exact message before the engine runs, and writes nothing.
#[test]
fn sim_bounds_and_receipt_paths_are_refused_before_the_engine() -> Check {
    for seed in 0..SEEDS {
        let mut rng = Seeded::from_seed(seed);
        let (rows, lines) = valid_set(&mut rng)?;
        let (extra, receipt, message) = illegal(&mut rng, rows.len())?;
        let mut args = vec![
            String::from("--phase"),
            String::from("pre-exec"),
            String::from("--query"),
            String::from("q"),
            String::from("--receipt"),
            receipt,
        ];
        for (flag, value) in extra {
            args.push(String::from(flag));
            args.push(value);
        }
        let request = parse(args)?;
        let root = Root::new()?;
        let mut stub = Stub {
            scores: vec![0.5; rows.len()],
            calls: 0,
            timeout: false,
        };
        let outcome = run(&request, lines.as_bytes(), &root.0, &mut stub);
        let Err(error) = outcome else {
            return Err(format!("seed {seed}: {message} was admitted").into());
        };
        assert_eq!(error.to_string(), message, "seed {seed}: the exact refusal");
        assert_eq!(stub.calls, 0, "seed {seed}: the engine never ran");
        let written = fs::read_dir(root.0.join("tools/receipts"))?.count();
        assert_eq!(written, 0, "seed {seed}: no receipt was written");
    }
    Ok(())
}

#[test]
fn a_timed_out_engine_and_a_bad_query_write_no_receipt() -> Check {
    let root = Root::new()?;
    let lines = "{\"id\":\"a\",\"text\":\"alpha\"}\n";
    let request = request("pre-exec", "late.json", &[])?;
    let mut stub = Stub {
        scores: vec![0.5],
        calls: 0,
        timeout: true,
    };
    let outcome = run(&request, lines.as_bytes(), &root.0, &mut stub);
    assert_eq!(
        outcome.map_err(|error| error.to_string()),
        Err(String::from("reranker exceeded its 7-second limit")),
        "the timeout surfaces"
    );
    assert!(
        !root.0.join("tools/receipts/late.json").exists(),
        "a timed-out run leaves no receipt"
    );
    for query in [String::new(), "q".repeat(16 * 1024 + 1)] {
        let request = parse(vec![
            String::from("--phase"),
            String::from("post-exec"),
            String::from("--query"),
            query,
            String::from("--receipt"),
            String::from("tools/receipts/q.json"),
        ])?;
        let outcome = run(&request, lines.as_bytes(), &root.0, &mut stub);
        assert_eq!(
            outcome.map_err(|error| error.to_string()),
            Err(String::from(
                "query must be non-empty and at most 16384 bytes"
            )),
            "the query bound"
        );
    }
    Ok(())
}

#[test]
fn the_command_line_refuses_unknown_repeated_and_missing_flags() -> Check {
    let cases: [(&[&str], &str); 6] = [
        (&["--nope", "x"], "usage: unknown argument --nope"),
        (
            &["--phase", "pre-exec", "--phase", "post-exec"],
            "usage: --phase given twice",
        ),
        (&["--query"], "usage: --query needs a value"),
        (
            &["--phase", "during", "--query", "q", "--receipt", "r"],
            "usage: --phase must be pre-exec or post-exec, not during",
        ),
        (
            &["--phase", "pre-exec", "--receipt", "r"],
            "usage: --query is required",
        ),
        (
            &[
                "--phase",
                "pre-exec",
                "--query",
                "q",
                "--receipt",
                "r",
                "--top-k",
                "-1",
            ],
            "usage: --top-k must be a whole number, not -1",
        ),
    ];
    for (args, message) in cases {
        let outcome = Request::parse(args.iter().map(|arg| OsString::from(*arg)));
        assert_eq!(
            outcome.map(|_| ()).map_err(|error| error.to_string()),
            Err(String::from(message)),
            "{args:?}"
        );
    }
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStringExt as _;
        let outcome = Request::parse([
            OsString::from("--query"),
            OsString::from_vec(vec![b'q', 0xff]),
        ]);
        assert!(
            matches!(outcome, Err(RerankError::Argument(_))),
            "a non-UTF-8 argument is refused as usage: {outcome:?}"
        );
    }
    Ok(())
}

/// Parses string arguments as the process would hand them over.
fn parse(args: Vec<String>) -> Result<Request, RerankError> {
    Request::parse(args.into_iter().map(OsString::from))
}
