//! `larry-rerank`: ranks JSONL candidates (`{"id": ..., "text": ...}` per line, from
//! `--candidates FILE` or standard input) against `--query` with the pinned local Qwen3 reranker,
//! writes the receipt named by `--receipt` under `tools/receipts/` create-exclusive, and prints it
//! as one JSON line (larry#150, ADR 0013). Built with `--features rerank`. A refusal prints
//! `larry-rerank: <message>` on standard error and exits 1; a usage refusal exits 2.
//!
//! ```text
//! larry-rerank --phase pre-exec|post-exec --query TEXT --receipt tools/receipts/NAME.json
//!              [--candidates FILE] [--top-k N] [--batch-size N] [--max-seconds N]
//! ```

use std::io::{self, Write as _};
use std::process::ExitCode;

use larry_demo::rerank::{self, Request, RerankError};

/// Parses the command line, runs the request from the working directory and prints the receipt.
fn rerank_once() -> Result<(), RerankError> {
    let request = Request::parse(std::env::args_os().skip(1))?;
    let root = std::env::current_dir().map_err(RerankError::Io)?;
    let line = rerank::rank(&request, io::stdin().lock(), &root)?;
    let mut out = io::stdout().lock();
    out.write_all(line.as_bytes())
        .and_then(|()| out.write_all(b"\n"))
        .and_then(|()| out.flush())
        .map_err(RerankError::Io)
}

fn main() -> ExitCode {
    let Err(error) = rerank_once() else {
        return ExitCode::SUCCESS;
    };
    let code = match error {
        RerankError::Usage(_) | RerankError::Argument(_) | RerankError::Number { .. } => 2,
        _ => 1,
    };
    if larry_demo::diagnose("larry-rerank", &error.to_string()).is_err() {
        return ExitCode::FAILURE;
    }
    ExitCode::from(code)
}
