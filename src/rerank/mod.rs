//! `larry-rerank`: ranks already-scoped JSONL candidates against one query with the pinned local
//! Qwen3 reranker and writes one create-exclusive receipt under `tools/receipts/` (larry#150,
//! ADR 0013). It ports `tools/rerank_local.py` with the same bounds and the same messages. The
//! request, bounds, ranking and receipt live here and build in every configuration; the model
//! engine is `qwen`, behind the `rerank` cargo feature, and [`rank`] is the one entry point.
//! Source: the Qwen3-Reranker model card (the yes/no judgment prompt), RFC 8259 (JSON text), the
//! Python tool it replaces.

use std::ffi::OsString;
use std::fmt;
use std::fs::{self, OpenOptions};
use std::io::{self, Read, Write};
use std::num::ParseIntError;
use std::path::{Path, PathBuf};
use std::str::Utf8Error;
use std::time::Instant;

use lgwks_std::hash;
use lgwks_std::json::{self, Value};

#[cfg(feature = "rerank")]
mod qwen;
#[cfg(test)]
mod sim;

/// The reranker model every receipt names.
pub(crate) const MODEL_ID: &str = "Qwen/Qwen3-Reranker-0.6B";
/// The pinned model revision every receipt names.
pub(crate) const MODEL_REVISION: &str = "e61197ed45024b0ed8a2d74b80b4d909f1255473";
/// Most candidates one request may rank.
const MAX_CANDIDATES: usize = 256;
/// Largest query, in bytes.
const MAX_QUERY_BYTES: usize = 16 * 1024;
/// Largest candidate text, in bytes.
const MAX_CANDIDATE_BYTES: usize = 256 * 1024;
/// Largest candidate input, in bytes.
const MAX_INPUT_BYTES: usize = 2 * 1024 * 1024;
/// Bytes read from the input per call.
const READ_CHUNK: usize = 64 * 1024;
/// The time limit when none is given, in seconds.
const DEFAULT_MAX_SECONDS: u64 = 900;
/// The largest time limit a request may set, in seconds.
const MAX_MAX_SECONDS: u64 = 3600;
/// Candidates scored per batch when none is given.
const DEFAULT_BATCH: usize = 2;
/// Results kept when no `--top-k` is given and there are at least this many candidates.
const DEFAULT_TOP_K: usize = 16;
/// Every flag the command line accepts; each takes one value.
const FLAGS: [&str; 7] = [
    "--phase",
    "--query",
    "--candidates",
    "--receipt",
    "--top-k",
    "--batch-size",
    "--max-seconds",
];

/// An engine failure's cause, kept for [`std::error::Error::source`].
pub(crate) type Cause = Box<dyn std::error::Error + Send + Sync>;

/// Why a request was refused or could not finish; each prints the Python tool's message.
#[derive(Debug)]
#[non_exhaustive]
pub enum RerankError {
    /// The command line is not one this tool reads.
    Usage(String),
    /// An argument is not UTF-8.
    Argument(OsString),
    /// A numeric flag's value is not a whole number.
    Number {
        /// The flag.
        flag: String,
        /// The value given.
        text: String,
        /// Why it did not parse.
        cause: ParseIntError,
    },
    /// The query is empty or past [`MAX_QUERY_BYTES`].
    Query,
    /// The candidate input is past [`MAX_INPUT_BYTES`].
    InputTooLarge,
    /// Fewer than one or more than [`MAX_CANDIDATES`] candidates.
    Count,
    /// A candidate line is not UTF-8.
    NotUtf8(Utf8Error),
    /// A candidate line is not JSON.
    Json(json::Error),
    /// A candidate is not a JSON object.
    NotObject,
    /// A candidate's `id` or `text` is missing, empty or not a string.
    Fields,
    /// A candidate's text is past [`MAX_CANDIDATE_BYTES`].
    TextTooLarge,
    /// Two candidates share an `id`.
    Duplicate,
    /// `--top-k`, `--batch-size` or `--max-seconds` is out of range.
    Bounds,
    /// The receipt is not directly under `tools/receipts/`; the cause when its directory could not
    /// be resolved.
    ReceiptPath(Option<io::Error>),
    /// Reading the input or writing the receipt failed.
    Io(io::Error),
    /// The engine ran past the request's time limit.
    Timeout(u64),
    /// The engine broke its own contract or found no model.
    Engine(String),
    /// The engine could not load or run the model.
    EngineFault {
        /// What the engine was doing.
        doing: &'static str,
        /// Why it failed.
        cause: Cause,
    },
}

impl fmt::Display for RerankError {
    fn fmt(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::Usage(ref detail) => write!(out, "usage: {detail}"),
            Self::Argument(ref arg) => write!(out, "usage: argument {arg:?} is not UTF-8"),
            Self::Number {
                ref flag, ref text, ..
            } => write!(out, "usage: {flag} must be a whole number, not {text}"),
            Self::Query => write!(
                out,
                "query must be non-empty and at most {MAX_QUERY_BYTES} bytes"
            ),
            Self::InputTooLarge => {
                write!(out, "candidate input exceeds {MAX_INPUT_BYTES} bytes")
            }
            Self::Count => write!(
                out,
                "candidate count must be between 1 and {MAX_CANDIDATES}"
            ),
            Self::NotUtf8(_) => out.write_str("candidate input is not UTF-8"),
            Self::Json(ref cause) => write!(out, "candidate line is not JSON: {cause}"),
            Self::NotObject => out.write_str("each candidate must be a JSON object"),
            Self::Fields => out.write_str("each candidate needs non-empty string id and text"),
            Self::TextTooLarge => {
                write!(out, "candidate text exceeds {MAX_CANDIDATE_BYTES} bytes")
            }
            Self::Duplicate => out.write_str("candidate ids must be unique"),
            Self::Bounds => out.write_str("top-k, batch-size and max-seconds must be bounded"),
            Self::ReceiptPath(_) => {
                out.write_str("receipt path must be directly under tools/receipts/")
            }
            Self::Io(ref cause) => write!(out, "{cause}"),
            Self::Timeout(seconds) => {
                write!(out, "reranker exceeded its {seconds}-second limit")
            }
            Self::Engine(ref detail) => write!(out, "reranker engine: {detail}"),
            Self::EngineFault { doing, ref cause } => {
                write!(out, "reranker engine: {doing}: {cause}")
            }
        }
    }
}

impl std::error::Error for RerankError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match *self {
            Self::Number { ref cause, .. } => Some(cause),
            Self::NotUtf8(ref cause) => Some(cause),
            Self::Json(ref cause) => Some(cause),
            Self::Io(ref cause) | Self::ReceiptPath(Some(ref cause)) => Some(cause),
            Self::EngineFault { ref cause, .. } => Some(&**cause),
            _ => None,
        }
    }
}

/// One candidate: a non-empty id and a non-empty text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Candidate {
    /// The caller's identifier, unique within the request.
    id: String,
    /// The passage the model judges.
    text: String,
}

impl Candidate {
    /// The passage the model judges.
    pub(crate) fn text(&self) -> &str {
        &self.text
    }
}

/// Which side of an execution a receipt records.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    /// Before the work runs.
    Pre,
    /// After the work ran.
    Post,
}

impl Phase {
    /// The receipt spelling.
    const fn as_str(self) -> &'static str {
        match self {
            Self::Pre => "pre-exec",
            Self::Post => "post-exec",
        }
    }
}

/// One parsed command line, before any input is read.
#[derive(Debug)]
pub struct Request {
    /// The receipt's phase.
    phase: Phase,
    /// The query every candidate is judged against.
    query: String,
    /// The candidate file, or standard input when absent.
    candidates: Option<PathBuf>,
    /// Where the receipt goes.
    receipt: PathBuf,
    /// How many results to keep, or the default when absent.
    top_k: Option<usize>,
    /// Candidates scored per batch.
    batch_size: usize,
    /// The engine's time limit.
    max_seconds: u64,
}

/// The engine: one probability of "yes" per candidate, in candidate order, finishing by
/// `deadline` or returning [`RerankError::Timeout`].
pub(crate) trait Scorer {
    /// Scores `rows` against `query`, `batch` at a time.
    fn score(
        &mut self,
        query: &str,
        rows: &[Candidate],
        batch: usize,
        deadline: Deadline,
    ) -> Result<Vec<f64>, RerankError>;
}

/// The instant a request's time limit runs out.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Deadline {
    /// When the request started.
    start: Instant,
    /// The limit, in seconds.
    seconds: u64,
}

impl Deadline {
    /// Refuses with [`RerankError::Timeout`] once the limit has passed.
    pub(crate) fn check(self) -> Result<(), RerankError> {
        if self.start.elapsed().as_secs() >= self.seconds {
            return Err(RerankError::Timeout(self.seconds));
        }
        Ok(())
    }
}

/// The parsed flag values, by flag.
struct Flags(Vec<(String, String)>);

impl Flags {
    /// The value of `name`, if given.
    fn take(&self, name: &str) -> Option<String> {
        self.0
            .iter()
            .find(|pair| pair.0 == name)
            .map(|pair| pair.1.clone())
    }

    /// The value of a required flag.
    fn required(&self, name: &str) -> Result<String, RerankError> {
        self.take(name)
            .ok_or_else(|| RerankError::Usage(format!("{name} is required")))
    }

    /// The decimal value of `name`, if given.
    fn number<T: std::str::FromStr<Err = ParseIntError>>(
        &self,
        name: &str,
    ) -> Result<Option<T>, RerankError> {
        match self.take(name) {
            None => Ok(None),
            Some(text) => match text.parse() {
                Ok(value) => Ok(Some(value)),
                Err(cause) => Err(RerankError::Number {
                    flag: String::from(name),
                    text,
                    cause,
                }),
            },
        }
    }
}

impl Request {
    /// Parses `--phase`, `--query`, `--candidates`, `--receipt`, `--top-k`, `--batch-size` and
    /// `--max-seconds` from the process arguments after the program name; each flag takes one
    /// value and appears at most once.
    pub fn parse<I: IntoIterator<Item = OsString>>(args: I) -> Result<Self, RerankError> {
        let mut flags: Vec<(String, String)> = Vec::new();
        let mut args = args.into_iter().map(|arg| arg.into_string());
        while let Some(flag) = args.next() {
            let flag = flag.map_err(RerankError::Argument)?;
            if !FLAGS.contains(&flag.as_str()) {
                return Err(RerankError::Usage(format!("unknown argument {flag}")));
            }
            if flags.iter().any(|pair| pair.0 == flag) {
                return Err(RerankError::Usage(format!("{flag} given twice")));
            }
            let value = match args.next() {
                Some(value) => value.map_err(RerankError::Argument)?,
                None => return Err(RerankError::Usage(format!("{flag} needs a value"))),
            };
            flags.push((flag, value));
        }
        let flags = Flags(flags);
        let phase = match flags.required("--phase")?.as_str() {
            "pre-exec" => Phase::Pre,
            "post-exec" => Phase::Post,
            other => {
                return Err(RerankError::Usage(format!(
                    "--phase must be pre-exec or post-exec, not {other}"
                )));
            }
        };
        let batch_size = match flags.number("--batch-size")? {
            Some(batch) => batch,
            None => DEFAULT_BATCH,
        };
        let max_seconds = match flags.number("--max-seconds")? {
            Some(seconds) => seconds,
            None => DEFAULT_MAX_SECONDS,
        };
        Ok(Self {
            phase,
            query: flags.required("--query")?,
            candidates: flags.take("--candidates").map(PathBuf::from),
            receipt: PathBuf::from(flags.required("--receipt")?),
            top_k: flags.number("--top-k")?,
            batch_size,
            max_seconds,
        })
    }
}

/// Reads `input` until it ends or holds more than [`MAX_INPUT_BYTES`], whichever comes first.
fn read_bounded<R: Read>(mut input: R) -> Result<Vec<u8>, RerankError> {
    let mut bytes = Vec::new();
    let mut chunk = vec![0_u8; READ_CHUNK];
    while bytes.len() <= MAX_INPUT_BYTES {
        let count = match input.read(&mut chunk) {
            Ok(0) => break,
            Ok(count) => count,
            Err(cause) if cause.kind() == io::ErrorKind::Interrupted => continue,
            Err(cause) => return Err(RerankError::Io(cause)),
        };
        let read = chunk
            .get(..count)
            .ok_or_else(|| RerankError::Engine(String::from("a read past its buffer")))?;
        bytes.extend_from_slice(read);
    }
    Ok(bytes)
}

/// Reads and validates JSONL candidates from `input`, in the Python tool's order: the input bound
/// and the count bound while reading, then each row's shape, then uniqueness.
pub(crate) fn candidates<R: Read>(input: R) -> Result<Vec<Candidate>, RerankError> {
    let bytes = read_bounded(input)?;
    let mut rows = Vec::new();
    let mut consumed = 0_usize;
    for line in bytes.split_inclusive(|&byte| byte == b'\n') {
        consumed = consumed.saturating_add(line.len());
        if consumed > MAX_INPUT_BYTES {
            return Err(RerankError::InputTooLarge);
        }
        let text = std::str::from_utf8(line).map_err(RerankError::NotUtf8)?;
        if text.trim().is_empty() {
            continue;
        }
        rows.push(json::from_str::<Value>(text).map_err(RerankError::Json)?);
        if rows.len() > MAX_CANDIDATES {
            return Err(RerankError::Count);
        }
    }
    if rows.is_empty() {
        return Err(RerankError::Count);
    }
    let mut normalized: Vec<Candidate> = Vec::with_capacity(rows.len());
    for row in &rows {
        if row.as_object().is_none() {
            return Err(RerankError::NotObject);
        }
        let field = |key: &str| {
            row.get(key)
                .and_then(Value::as_str)
                .filter(|value| !value.is_empty())
                .map(String::from)
        };
        let (Some(id), Some(text)) = (field("id"), field("text")) else {
            return Err(RerankError::Fields);
        };
        if text.len() > MAX_CANDIDATE_BYTES {
            return Err(RerankError::TextTooLarge);
        }
        normalized.push(Candidate { id, text });
    }
    let mut ids: Vec<&str> = normalized.iter().map(|row| row.id.as_str()).collect();
    ids.sort_unstable();
    if ids
        .windows(2)
        .any(|pair| matches!(*pair, [first, second] if first == second))
    {
        return Err(RerankError::Duplicate);
    }
    Ok(normalized)
}

/// A validated request: the bounds hold and the receipt has a legal home.
#[derive(Debug)]
struct Admitted {
    /// The candidates, in input order.
    rows: Vec<Candidate>,
    /// How many results the receipt keeps.
    top_k: usize,
    /// The receipt's absolute path.
    receipt: PathBuf,
}

/// Checks the bounds and the receipt path against `root`, the working directory.
fn admit(request: &Request, rows: Vec<Candidate>, root: &Path) -> Result<Admitted, RerankError> {
    let top_k = match request.top_k {
        Some(top_k) => top_k,
        None => rows.len().min(DEFAULT_TOP_K),
    };
    let bounded = (1..=rows.len()).contains(&top_k)
        && (1..=MAX_CANDIDATES).contains(&request.batch_size)
        && (1..=MAX_MAX_SECONDS).contains(&request.max_seconds);
    if !bounded {
        return Err(RerankError::Bounds);
    }
    let home = root.join("tools").join("receipts");
    fs::create_dir_all(&home).map_err(RerankError::Io)?;
    let home = fs::canonicalize(&home).map_err(RerankError::Io)?;
    let given = if request.receipt.is_absolute() {
        request.receipt.clone()
    } else {
        root.join(&request.receipt)
    };
    let (Some(parent), Some(name)) = (given.parent(), given.file_name()) else {
        return Err(RerankError::ReceiptPath(None));
    };
    let parent = fs::canonicalize(parent).map_err(|cause| RerankError::ReceiptPath(Some(cause)))?;
    if parent != home {
        return Err(RerankError::ReceiptPath(None));
    }
    Ok(Admitted {
        rows,
        top_k,
        receipt: home.join(name),
    })
}

/// The candidates as the receipt digests them: compact JSON, keys sorted, text unescaped.
fn canonical(rows: &[Candidate]) -> Result<String, RerankError> {
    let value = Value::Array(
        rows.iter()
            .map(|row| json::json!({"id": row.id, "text": row.text()}))
            .collect(),
    );
    json::to_string(&value).map_err(RerankError::Json)
}

/// Runs one request with the pinned local model: the candidates from the request's file or
/// `input`, the receipt under `root/tools/receipts/`; returns the receipt's JSON line. A build
/// without the `rerank` feature refuses after every bound has been checked.
pub fn rank<R: Read>(request: &Request, input: R, root: &Path) -> Result<String, RerankError> {
    #[cfg(feature = "rerank")]
    let mut scorer = qwen::Qwen::from_cache()?;
    #[cfg(not(feature = "rerank"))]
    let mut scorer = Unbuilt;
    run(request, input, root, &mut scorer)
}

/// The engine of a build without the `rerank` feature.
#[cfg(not(feature = "rerank"))]
struct Unbuilt;

#[cfg(not(feature = "rerank"))]
impl Scorer for Unbuilt {
    fn score(
        &mut self,
        _query: &str,
        _rows: &[Candidate],
        _batch: usize,
        _deadline: Deadline,
    ) -> Result<Vec<f64>, RerankError> {
        Err(RerankError::Engine(String::from(
            "this build has no model; rebuild with --features rerank",
        )))
    }
}

/// Runs one request end to end: validates the query, reads the candidates from the request's file
/// or `input`, checks the bounds and receipt path against `root`, scores with `scorer`, refuses a
/// run past its time limit, writes the receipt create-exclusive and returns its JSON line.
pub(crate) fn run<S: Scorer, R: Read>(
    request: &Request,
    input: R,
    root: &Path,
    scorer: &mut S,
) -> Result<String, RerankError> {
    let deadline = Deadline {
        start: Instant::now(),
        seconds: request.max_seconds,
    };
    if request.query.is_empty() || request.query.len() > MAX_QUERY_BYTES {
        return Err(RerankError::Query);
    }
    let rows = match request.candidates {
        Some(ref path) => candidates(fs::File::open(path).map_err(RerankError::Io)?)?,
        None => candidates(input)?,
    };
    let admitted = admit(request, rows, root)?;
    let scores = scorer.score(&request.query, &admitted.rows, request.batch_size, deadline)?;
    deadline.check()?;
    if scores.len() != admitted.rows.len() {
        return Err(RerankError::Engine(format!(
            "{} scores for {} candidates",
            scores.len(),
            admitted.rows.len()
        )));
    }
    let mut ranked: Vec<(&Candidate, f64)> = admitted.rows.iter().zip(scores).collect();
    ranked.sort_by(|left, right| right.1.total_cmp(&left.1));
    let results: Vec<Value> = ranked
        .iter()
        .take(admitted.top_k)
        .map(|&(row, score)| json::json!({"id": row.id, "score": score}))
        .collect();
    let receipt = json::json!({
        "phase": request.phase.as_str(),
        "model": MODEL_ID,
        "revision": MODEL_REVISION,
        "query_blake3": hash::blake3(request.query.as_bytes()).to_hex(),
        "candidates_blake3": hash::blake3(canonical(&admitted.rows)?.as_bytes()).to_hex(),
        "candidate_count": admitted.rows.len(),
        "top_k": admitted.top_k,
        "max_seconds": request.max_seconds,
        "results": results,
    });
    let line = json::to_string(&receipt).map_err(RerankError::Json)?;
    write_exclusive(&admitted.receipt, &line)?;
    Ok(line)
}

/// Writes `line` and a newline to a file that must not exist yet, owner read-write only.
fn write_exclusive(path: &Path, line: &str) -> Result<(), RerankError> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.mode(0o600);
    }
    let mut file = options.open(path).map_err(RerankError::Io)?;
    file.write_all(line.as_bytes()).map_err(RerankError::Io)?;
    file.write_all(b"\n").map_err(RerankError::Io)?;
    file.sync_all().map_err(RerankError::Io)
}
