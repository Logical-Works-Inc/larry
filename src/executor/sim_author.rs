//! Seeded simulations of the brief parser and the write guards (larry#123).
//! Source: POSIX path resolution (a symlink component redirects every later
//! component) and CWE-59 (link following). Requirement: every malformed brief
//! gets its exact error, every valid one round-trips, a symlink at any depth of
//! the clone is never written through, and a hostile reply is written exactly as
//! the reference model says. One seed drives every path, depth and byte.

use std::error::Error;
use std::fs;
use std::path::PathBuf;

use lgwks_std::json::{Map, Value, json};
use lgwks_std::seeded::Seeded;

use super::{AuthorFault, Brief, MAX_FILES, MAX_GOAL_BYTES, checked_target, file_text};
use crate::executor::{ExecError, TaskError};
use crate::sim_error::SimError;

/// What every test returns.
type TestResult = Result<(), Box<dyn Error>>;

/// The seeds every simulation runs.
const SEEDS: u64 = 64;

/// The document `{goal, files}` as a map.
fn document(goal: &Value, files: &Value) -> Result<Map, SimError> {
    let mut map = Map::new();
    map.insert("goal".to_owned(), goal.clone());
    map.insert("files".to_owned(), files.clone());
    Ok(map)
}

/// A seeded valid path: one to four lowercase components.
fn valid_path(rng: &mut Seeded) -> Result<String, SimError> {
    let depth = rng.below(4)?.saturating_add(1);
    let mut parts = Vec::new();
    for _ in 0..depth {
        let length = usize::try_from(rng.below(8)?.saturating_add(1))?;
        let mut part = String::with_capacity(length);
        for _ in 0..length {
            part.push(char::from(
                b'a'.saturating_add(u8::try_from(rng.below(26)?)?),
            ));
        }
        parts.push(part);
    }
    Ok(parts.join("/"))
}

/// A fresh directory under the system temp dir for one test.
pub(super) fn scratch(name: &str) -> Result<PathBuf, SimError> {
    let path = std::env::temp_dir().join(format!("larry-author-{name}-{}", std::process::id()));
    match fs::remove_dir_all(&path) {
        Ok(()) => {}
        Err(cause) if cause.kind() == std::io::ErrorKind::NotFound => {}
        Err(cause) => return Err(SimError::Io(cause)),
    }
    fs::create_dir_all(&path)?;
    Ok(path)
}

#[test]
fn sim_valid_briefs_roundtrip_for_every_seed() -> TestResult {
    for seed in 0..SEEDS {
        let mut rng = Seeded::from_seed(seed);
        let count = usize::try_from(rng.below(u64::try_from(MAX_FILES)?)?.saturating_add(1))?;
        let mut files: Vec<String> = Vec::with_capacity(count);
        while files.len() < count {
            let path = format!("{}{}", valid_path(&mut rng)?, files.len());
            files.push(path);
        }
        let goal = format!("goal {seed}");
        let brief = Brief::parse(&document(&json!(goal), &json!(files))?)?
            .ok_or("a goal and files make a brief")?;
        assert_eq!(brief.goal, goal, "seed {seed}: the goal reads back");
        assert_eq!(
            brief.files, files,
            "seed {seed}: the files read back in order"
        );
    }
    assert!(
        Brief::parse(&Map::new())?.is_none(),
        "neither key is no brief"
    );
    Ok(())
}

#[test]
fn sim_malformed_briefs_get_their_exact_error_for_every_seed() -> TestResult {
    for seed in 0..SEEDS {
        let mut rng = Seeded::from_seed(seed);
        let good = valid_path(&mut rng)?;
        let goal = json!("bound the retry loop");
        let only_goal = json!({"goal": "g"}).as_object().ok_or("object")?.clone();
        assert!(matches!(
            Brief::parse(&only_goal),
            Err(TaskError::IncompleteBrief)
        ));
        assert!(matches!(
            Brief::parse(&document(&json!(7), &json!([good]))?),
            Err(TaskError::WrongType { key: "goal" })
        ));
        for bad_goal in ["", "  \n", "a\0b"] {
            assert!(matches!(
                Brief::parse(&document(&json!(bad_goal), &json!([good]))?),
                Err(TaskError::BadGoal)
            ));
        }
        let long_goal = "g".repeat(MAX_GOAL_BYTES.saturating_add(1));
        assert!(matches!(
            Brief::parse(&document(&json!(long_goal), &json!([good]))?),
            Err(TaskError::BadGoal)
        ));
        assert!(matches!(
            Brief::parse(&document(&goal, &json!("README"))?),
            Err(TaskError::WrongType { key: "files" })
        ));
        assert!(matches!(
            Brief::parse(&document(&goal, &json!([]))?),
            Err(TaskError::FileCount { count: 0 })
        ));
        let too_many: Vec<String> = (0..=MAX_FILES).map(|index| format!("f{index}")).collect();
        assert!(matches!(
            Brief::parse(&document(&goal, &json!(too_many))?),
            Err(TaskError::FileCount { count }) if count == MAX_FILES.saturating_add(1)
        ));
        let escapes = [
            format!("../{good}"),
            format!("/{good}"),
            format!("./{good}"),
            format!("{good}/../x"),
            format!(".git/{good}"),
            format!("{good}/.git"),
            format!("{good}\0"),
            String::new(),
            "p".repeat(1025),
        ];
        for escape in &escapes {
            assert!(
                matches!(
                    Brief::parse(&document(&goal, &json!([good, escape]))?),
                    Err(TaskError::BadFile { index: 1 })
                ),
                "seed {seed}: {escape:?} is refused at its index"
            );
        }
        assert!(matches!(
            Brief::parse(&document(&goal, &json!([good, good]))?),
            Err(TaskError::BadFile { index: 1 })
        ));
        assert!(matches!(
            Brief::parse(&document(&goal, &json!([good, 3]))?),
            Err(TaskError::WrongType { key: "files" })
        ));
    }
    Ok(())
}

/// Hostile reply pieces: NUL, a right-to-left override, a zero-width space, CRLF,
/// an emoji, stray backticks and a whole fence inside the body.
const PIECES: [&str; 12] = [
    "a", "\0", "\u{202e}", "\u{200b}", "\r\n", "\n", "😀", "`", "```", " ", "\t", "é",
];

/// A seeded body of up to 11 hostile pieces.
fn hostile_body(rng: &mut Seeded) -> Result<String, SimError> {
    let mut body = String::new();
    for _ in 0..rng.below(12)? {
        body.push_str(PIECES[usize::try_from(rng.below(12)?)?]);
    }
    Ok(body)
}

/// The reference model of `file_text`: what the file must hold for `reply`.
fn with_newline(text: &str) -> String {
    if text.is_empty() || text.ends_with('\n') {
        text.to_owned()
    } else {
        format!("{text}\n")
    }
}

/// weird-bytes and liar-upstream: a gateway replies with hostile bytes raw, fenced,
/// fenced inside whitespace or with the closing fence cut off. The file holds
/// exactly what the reference model says, never more than the reply plus one
/// newline, and always ends in a newline unless empty. Catches a fence stripper
/// that eats body bytes, strips an unclosed fence, or trims inside the body.
#[test]
fn sim_author_weird_bytes_replies_are_written_as_the_reference_model_says() -> TestResult {
    for seed in 0..SEEDS.saturating_mul(16) {
        let mut rng = Seeded::from_seed(seed);
        let lang = ["", "rust", "md", "sh"][usize::try_from(rng.below(4)?)?];
        let raw = hostile_body(&mut rng)?;
        let body = with_newline(&raw);
        let (reply, want) = match rng.below(4)? {
            0 => {
                let reply = format!("x{raw}");
                let want = with_newline(&reply);
                (reply, want)
            }
            1 => (format!("```{lang}\n{body}```"), body.clone()),
            2 => (format!(" \n\t```{lang}\n{body}```\r\n "), body.clone()),
            _ => {
                let reply = format!("```{lang}\n{raw}x");
                let want = with_newline(&reply);
                (reply, want)
            }
        };
        let written = file_text(&reply);
        assert_eq!(written, want, "seed {seed}: reply {reply:?}");
        assert!(
            written.len() <= reply.len().saturating_add(1),
            "seed {seed}: never grows past one newline"
        );
        assert!(
            written.is_empty() || written.ends_with('\n'),
            "seed {seed}: ends in a newline"
        );
    }
    Ok(())
}

/// saboteur and neighbour: a clone carries a symlink at a seeded depth of a seeded
/// path, pointing at a neighbour's directory or at nothing (dangling). The guard
/// refuses exactly when the reference model placed a link, and the neighbour's
/// directory stays empty. Catches a guard that checks only the leaf, follows a
/// dangling link, or uses `metadata` (which follows links) instead of `symlink_metadata`.
#[cfg(unix)]
#[test]
fn sim_author_saboteur_symlinks_at_any_depth_are_refused_exactly() -> TestResult {
    let neighbour = scratch("neighbour")?;
    for seed in 0..SEEDS {
        let mut rng = Seeded::from_seed(seed);
        let tree = scratch(&format!("sabotage-{seed}"))?;
        let path = valid_path(&mut rng)?;
        let parts: Vec<&str> = path.split('/').collect();
        let depth = usize::try_from(rng.below(u64::try_from(parts.len())?.saturating_add(1))?)?;
        let mut prefix = tree.clone();
        for part in parts.iter().take(depth) {
            prefix.push(part);
            fs::create_dir_all(&prefix)?;
        }
        let linked = depth < parts.len();
        if let Some(part) = parts.get(depth) {
            let target = if rng.below(2)? == 0 {
                neighbour.clone()
            } else {
                neighbour.join("gone")
            };
            std::os::unix::fs::symlink(target, prefix.join(part))?;
        }
        let outcome = checked_target(&tree, &path);
        if linked {
            assert!(
                matches!(
                    outcome,
                    Err(ExecError::Author {
                        fault: AuthorFault::Symlink,
                        ..
                    })
                ),
                "seed {seed}: a link at component {depth} of {path} is refused"
            );
        } else {
            assert_eq!(
                outcome?,
                tree.join(&path),
                "seed {seed}: a link-free path is joined"
            );
        }
        assert_eq!(
            fs::read_dir(&neighbour)?.count(),
            0,
            "seed {seed}: the neighbour is untouched"
        );
        fs::remove_dir_all(&tree)?;
    }
    fs::remove_dir_all(&neighbour)?;
    Ok(())
}
