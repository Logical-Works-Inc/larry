//! Faults at the public edge of `larry-rerank`: an input stream that is interrupted, fails or
//! lies, a candidate file that is missing, a receipt path with no legal home, and every refusal's
//! message and cause. Source: the `std::io::Read` contract (`Interrupted` is retried), CWE-20,
//! CWE-22 and `std::error::Error::source`. Requirement: each fault is its own typed refusal, an
//! interrupted read loses nothing, every bound is checked before the model, and no refused run
//! leaves a receipt.

#[path = "support/temp_dir.rs"]
mod temp_dir;

use std::error::Error as _;
use std::ffi::OsString;

use larry_demo::rerank::RerankError;
use lgwks_std::json::{self, Value};

/// Failures propagate instead of panicking.
type Check<T = ()> = Result<T, Box<dyn std::error::Error>>;

/// Each refusal with the message a person reads and the cause kept for `source`.
fn refusals() -> Check<Vec<(RerankError, String, Option<String>)>> {
    let Err(number) = "x7".parse::<u64>() else {
        return Err("x7 parsed".into());
    };
    let Err(utf8) = std::str::from_utf8(&Vec::from([0xff_u8])).map(str::len) else {
        return Err("0xff decoded".into());
    };
    let syntax = || match json::from_str::<Value>("{") {
        Ok(value) => Err(format!("a lone brace parsed as {value}")),
        Err(cause) => Ok(cause),
    };
    let gone = || std::io::Error::other("the disk went away");
    let plain = |error: RerankError, message: &str| (error, String::from(message), None);
    let caused = |error: RerankError, message: String, cause: String| (error, message, Some(cause));
    let receipt = "receipt path must be directly under tools/receipts/";
    Ok(vec![
        plain(RerankError::Usage(String::from("no")), "usage: no"),
        plain(
            RerankError::Argument(OsString::from("q")),
            "usage: argument \"q\" is not UTF-8",
        ),
        caused(
            RerankError::Number {
                flag: String::from("--top-k"),
                text: String::from("x7"),
                cause: number.clone(),
            },
            String::from("usage: --top-k must be a whole number, not x7"),
            number.to_string(),
        ),
        plain(
            RerankError::Query,
            "query must be non-empty and at most 16384 bytes",
        ),
        plain(
            RerankError::InputTooLarge,
            "candidate input exceeds 2097152 bytes",
        ),
        plain(
            RerankError::Count,
            "candidate count must be between 1 and 256",
        ),
        caused(
            RerankError::NotUtf8(utf8),
            String::from("candidate input is not UTF-8"),
            utf8.to_string(),
        ),
        caused(
            RerankError::Json(syntax()?),
            format!("candidate line is not JSON: {}", syntax()?),
            syntax()?.to_string(),
        ),
        plain(
            RerankError::NotObject,
            "each candidate must be a JSON object",
        ),
        plain(
            RerankError::Fields,
            "each candidate needs non-empty string id and text",
        ),
        plain(
            RerankError::TextTooLarge,
            "candidate text exceeds 262144 bytes",
        ),
        plain(RerankError::Duplicate, "candidate ids must be unique"),
        plain(
            RerankError::Bounds,
            "top-k, batch-size and max-seconds must be bounded",
        ),
        plain(RerankError::ReceiptPath(None), receipt),
        caused(
            RerankError::ReceiptPath(Some(gone())),
            String::from(receipt),
            gone().to_string(),
        ),
        caused(
            RerankError::Io(gone()),
            gone().to_string(),
            gone().to_string(),
        ),
        plain(
            RerankError::Timeout(9),
            "reranker exceeded its 9-second limit",
        ),
        plain(
            RerankError::Engine(String::from("no model")),
            "reranker engine: no model",
        ),
        caused(
            RerankError::EngineFault {
                doing: "loading",
                cause: Box::new(gone()),
            },
            format!("reranker engine: loading: {}", gone()),
            gone().to_string(),
        ),
        plain(
            RerankError::Pin,
            "the model pin must be a model id line then a 40-hex revision line",
        ),
    ])
}

#[test]
fn every_refusal_prints_its_message_and_keeps_its_cause() -> Check {
    for (error, message, cause) in refusals()? {
        assert_eq!(error.to_string(), message, "{error:?}: the message");
        assert_eq!(
            error.source().map(ToString::to_string),
            cause,
            "{error:?}: the cause"
        );
    }
    Ok(())
}

/// The runs below stop at the missing model, which is what proves every earlier step passed.
/// A build with the model would load weights instead, so they are compiled without it.
#[cfg(not(feature = "rerank"))]
mod unbuilt {
    use std::ffi::OsString;
    use std::fs;
    use std::io::{self, Read};
    use std::path::Path;

    use larry_demo::rerank::{Request, RerankError, rank};
    use lgwks_std::seeded::Seeded;

    use super::{Check, temp_dir::TempDir};

    /// Seeds each fault runs.
    const SEEDS: u64 = 64;

    /// Two candidates, as the input stream carries them.
    const TWO: &str = "{\"id\":\"a\",\"text\":\"alpha\"}\n{\"id\":\"b\",\"text\":\"beta\"}\n";

    /// What a faulty reader does once it has handed over its first `fault_at` bytes.
    #[derive(Debug, Clone, Copy)]
    enum Fault {
        /// One `Interrupted`, then the rest of the stream.
        Interrupt,
        /// The error kind, for good.
        Fail(io::ErrorKind),
        /// A count one past the buffer it was handed.
        Overrun,
    }

    /// A stream handed over in pieces, with one fault part way.
    struct Faulty<'a> {
        /// Bytes not yet handed over.
        left: &'a [u8],
        /// Bytes still to hand over before the fault.
        fault_at: usize,
        /// The fault, until an interrupt has happened.
        fault: Option<Fault>,
        /// The largest piece one call hands over.
        piece: usize,
    }

    impl Read for Faulty<'_> {
        fn read(&mut self, into: &mut [u8]) -> io::Result<usize> {
            if self.fault_at == 0 {
                match self.fault {
                    Some(Fault::Interrupt) => {
                        self.fault = None;
                        return Err(io::Error::from(io::ErrorKind::Interrupted));
                    }
                    Some(Fault::Fail(kind)) => return Err(io::Error::from(kind)),
                    Some(Fault::Overrun) => return Ok(into.len().saturating_add(1)),
                    None => {}
                }
            }
            let mut count = self.piece.min(into.len()).min(self.left.len());
            if self.fault.is_some() {
                count = count.min(self.fault_at);
            }
            let Some((head, tail)) = self.left.split_at_checked(count) else {
                return Err(io::Error::from(io::ErrorKind::InvalidData));
            };
            for (slot, byte) in into.iter_mut().zip(head) {
                *slot = *byte;
            }
            self.left = tail;
            self.fault_at = self.fault_at.saturating_sub(count);
            Ok(count)
        }
    }

    /// A request naming `receipt`, keeping `top_k` results, with `extra` flags after it.
    fn request(receipt: &str, top_k: &str, extra: &[(&str, &Path)]) -> Check<Request> {
        let mut args: Vec<OsString> = [
            "--phase",
            "pre-exec",
            "--query",
            "retry budget",
            "--receipt",
            receipt,
            "--top-k",
            top_k,
        ]
        .into_iter()
        .map(OsString::from)
        .collect();
        for &(flag, value) in extra {
            args.push(OsString::from(flag));
            args.push(value.as_os_str().to_owned());
        }
        Ok(Request::parse(args)?)
    }

    /// Whether `outcome` got past every bound to the missing model.
    fn reached_the_model(outcome: &Result<String, RerankError>) -> bool {
        matches!(
            *outcome,
            Err(RerankError::Engine(ref detail))
                if detail == "this build has no model; rebuild with --features rerank"
        )
    }

    /// Files in the root's receipt directory; none when it was never made.
    fn receipts(root: &Path) -> Check<usize> {
        match fs::read_dir(root.join("tools/receipts")) {
            Ok(entries) => Ok(entries.count()),
            Err(cause) if cause.kind() == io::ErrorKind::NotFound => Ok(0),
            Err(cause) => Err(cause.into()),
        }
    }

    // `--top-k 2` is only in bounds when both rows arrived, so a lost piece reads as `Bounds`.
    #[test]
    fn sim_an_interrupted_read_loses_nothing_and_a_failed_or_lying_one_is_refused() -> Check {
        let kinds = [
            io::ErrorKind::BrokenPipe,
            io::ErrorKind::ConnectionReset,
            io::ErrorKind::UnexpectedEof,
            io::ErrorKind::PermissionDenied,
        ];
        let root = TempDir::new()?;
        let named = request("tools/receipts/fault.json", "2", &[])?;
        for seed in 0..SEEDS {
            let mut rng = Seeded::from_seed(seed);
            let fault_at = rng.index(TWO.len())?;
            let piece = rng.index(17)?.saturating_add(1);
            let through = |fault: Fault| {
                let reader = Faulty {
                    left: TWO.as_bytes(),
                    fault_at,
                    fault: Some(fault),
                    piece,
                };
                rank(&named, reader, root.path())
            };
            let resumed = through(Fault::Interrupt);
            assert!(
                reached_the_model(&resumed),
                "seed {seed}: an interrupted read is retried: {resumed:?}"
            );
            for kind in kinds {
                let failed = through(Fault::Fail(kind));
                assert!(
                    matches!(failed, Err(RerankError::Io(ref cause)) if cause.kind() == kind),
                    "seed {seed}: {kind:?} is kept: {failed:?}"
                );
            }
            let lied = through(Fault::Overrun);
            assert!(
                matches!(
                    lied,
                    Err(RerankError::Engine(ref detail)) if detail == "a read past its buffer"
                ),
                "seed {seed}: a count past the buffer is refused: {lied:?}"
            );
        }
        assert_eq!(receipts(root.path())?, 0, "no fault leaves a receipt");
        Ok(())
    }

    #[test]
    fn a_candidate_file_is_read_instead_of_the_stream_and_a_missing_one_is_refused() -> Check {
        let root = TempDir::new()?;
        let given = root.path().join("rows.jsonl");
        fs::write(&given, TWO)?;
        let named = request("tools/receipts/file.json", "2", &[("--candidates", &given)])?;
        let read = rank(&named, "not json".as_bytes(), root.path());
        assert!(
            reached_the_model(&read),
            "the rows came from the file: {read:?}"
        );

        let gone = root.path().join("gone.jsonl");
        let missing = request("tools/receipts/gone.json", "2", &[("--candidates", &gone)])?;
        let refused = rank(&missing, TWO.as_bytes(), root.path());
        assert!(
            matches!(
                refused,
                Err(RerankError::Io(ref cause)) if cause.kind() == io::ErrorKind::NotFound
            ),
            "a missing file never falls back to the stream: {refused:?}"
        );
        assert_eq!(receipts(root.path())?, 0, "neither run writes");
        Ok(())
    }

    #[test]
    fn a_receipt_with_no_legal_home_and_an_unmet_bound_are_refused_before_the_model() -> Check {
        let root = TempDir::new()?;
        for receipt in ["/", "tools/receipts/..", "tools/beside.json"] {
            let named = request(receipt, "2", &[])?;
            let refused = rank(&named, TWO.as_bytes(), root.path());
            assert!(
                matches!(refused, Err(RerankError::ReceiptPath(None))),
                "{receipt}: not a file under the receipts: {refused:?}"
            );
        }
        let unmade = request("tools/missing/receipt.json", "2", &[])?;
        let refused = rank(&unmade, TWO.as_bytes(), root.path());
        assert!(
            matches!(
                refused,
                Err(RerankError::ReceiptPath(Some(ref cause)))
                    if cause.kind() == io::ErrorKind::NotFound
            ),
            "an unmade directory keeps its cause: {refused:?}"
        );
        let three = request("tools/receipts/three.json", "3", &[])?;
        let bounded = rank(&three, TWO.as_bytes(), root.path());
        assert!(
            matches!(bounded, Err(RerankError::Bounds)),
            "one past the rows is refused before the missing model: {bounded:?}"
        );
        assert_eq!(receipts(root.path())?, 0, "no refusal writes");
        Ok(())
    }
}
