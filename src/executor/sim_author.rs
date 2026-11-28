//! Seeded simulations of the brief parser and the write guards (larry#123).
//! Source: POSIX path resolution (a symlink component redirects every later
//! component) and CWE-59 (link following). Requirement: every malformed brief
//! gets its exact error, every valid one round-trips, a symlink at any depth of
//! the clone is never written through, and a hostile reply is written exactly as
//! the reference model says. One seed drives every path, depth and byte.

use std::error::Error;
use std::fs;
use std::time::Duration;

use lgwks_bot::kernel::clock::Clock;
use lgwks_std::json::{Map, Value, json};
use lgwks_std::seeded::Seeded;

use super::{
    AuthorFault, Brief, MAX_CONTEXT_BYTES, MAX_FILES, MAX_GOAL_BYTES, SYSTEM, checked_target,
    file_text, fitting_context, prefix_label, reply_file, shared_context, system_line, user_turn,
};
use crate::executor::{ExecError, TaskError};
use crate::model::{Draft, Kind, Message, ModelClient, ModelError, Role, Route, sim_config};
use crate::sim_error::SimError;

/// The simulations of a brief's source read (larry#617).
#[path = "sim_author_source.rs"]
mod source;

/// What every test returns.
type TestResult = Result<(), Box<dyn Error>>;

/// The seeds every simulation runs.
const SEEDS: u64 = 64;

/// The seeds the prompt-prefix simulations run (larry#230).
const CACHE_SEEDS: u64 = 256;

/// A route the prompt-prefix simulations share: an OpenAI chat route with no
/// template, so the system line is the standing instruction and the partition.
fn openai_route() -> Result<Route, crate::model::RouteError> {
    Route::new(&Draft {
        id: "sim",
        kind: Kind::OpenAiChat,
        base_url: "http://127.0.0.1:9/v1",
        model: "sim-model",
        key_env: "LARRY_KEY_SIM",
        temperature: None,
        request_bytes: None,
        residency: None,
        prompt: None,
        describe: "a route the simulation never sends on",
    })
}
/// The document `{goal, files}` as a map.
fn document(goal: &Value, files: &Value) -> Map {
    let mut map = Map::new();
    map.insert("goal".to_owned(), goal.clone());
    map.insert("files".to_owned(), files.clone());
    map
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

/// A fresh directory under the system temp dir for one test: empty when it is made and removed
/// when its guard drops, so a process id the system reuses never lands a test in a stale tree
/// (larry#267).
pub(super) fn scratch(name: &str) -> Result<crate::executor::sim_tree::Scratch, SimError> {
    crate::executor::sim_tree::scratch(&format!("larry-author-{name}"))
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
        let brief = Brief::parse(&document(&json!(goal), &json!(files)))?
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
            Brief::parse(&document(&json!(7), &json!([good]))),
            Err(TaskError::WrongType { key: "goal" })
        ));
        for bad_goal in ["", "  \n", "a\0b"] {
            assert!(matches!(
                Brief::parse(&document(&json!(bad_goal), &json!([good]))),
                Err(TaskError::BadGoal)
            ));
        }
        let long_goal = "g".repeat(MAX_GOAL_BYTES.saturating_add(1));
        assert!(matches!(
            Brief::parse(&document(&json!(long_goal), &json!([good]))),
            Err(TaskError::BadGoal)
        ));
        assert!(matches!(
            Brief::parse(&document(&goal, &json!("README"))),
            Err(TaskError::WrongType { key: "files" })
        ));
        assert!(matches!(
            Brief::parse(&document(&goal, &json!([]))),
            Err(TaskError::FileCount { count: 0 })
        ));
        let too_many: Vec<String> = (0..=MAX_FILES).map(|index| format!("f{index}")).collect();
        assert!(matches!(
            Brief::parse(&document(&goal, &json!(too_many))),
            Err(TaskError::FileCount { count }) if count == MAX_FILES.saturating_add(1)
        ));
        let escapes = [
            format!("../{good}"),
            format!("/{good}"),
            format!("./{good}"),
            format!("{good}/../x"),
            format!(".git/{good}"),
            format!("{good}/.git"),
            String::from(".GIT/hooks/x"),
            String::from(".Git/config"),
            format!("{good}/.GIT"),
            format!("{good}\0"),
            String::new(),
            "p".repeat(1025),
        ];
        for escape in &escapes {
            assert!(
                matches!(
                    Brief::parse(&document(&goal, &json!([good, escape]))),
                    Err(TaskError::BadFile { index: 1 })
                ),
                "seed {seed}: {escape:?} is refused at its index"
            );
        }
        assert!(matches!(
            Brief::parse(&document(&goal, &json!([good, good]))),
            Err(TaskError::BadFile { index: 1 })
        ));
        assert!(matches!(
            Brief::parse(&document(&goal, &json!([good, 3]))),
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
        body.push_str(
            PIECES
                .get(usize::try_from(rng.below(12)?)?)
                .ok_or(SimError::Missing("hostile pieces"))?,
        );
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
        let lang = *["", "rust", "md", "sh"]
            .get(usize::try_from(rng.below(4)?)?)
            .ok_or("languages")?;
        let raw = hostile_body(&mut rng)?;
        let body = with_newline(&raw);
        let (reply, want) = match rng.below(4)? {
            0 => {
                let reply = format!("x{raw}");
                let want = with_newline(&reply);
                (reply, want)
            }
            1 => (format!("```{lang}\n{body}```"), body),
            2 => (format!(" \n\t```{lang}\n{body}```\r\n "), body),
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
        let mut prefix = tree.to_path_buf();
        for part in parts.iter().take(depth) {
            prefix.push(part);
            fs::create_dir_all(&prefix)?;
        }
        let linked = depth < parts.len();
        if let Some(part) = parts.get(depth) {
            let target = if rng.below(2)? == 0 {
                neighbour.to_path_buf()
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
    }
    Ok(())
}

/// A seeded copy of `items` in a seeded order (Fisher-Yates).
fn shuffled<Item: Clone>(rng: &mut Seeded, items: &[Item]) -> Result<Vec<Item>, SimError> {
    let mut mixed = items.to_vec();
    for index in (1..mixed.len()).rev() {
        let other = rng.index(index.saturating_add(1))?;
        mixed.swap(index, other);
    }
    Ok(mixed)
}

/// One seeded brief: `count` distinct paths in a seeded order, each an existing file of
/// hostile and ASCII text or a file that does not exist yet.
fn brief_files(rng: &mut Seeded, count: usize) -> Result<Vec<(String, Option<String>)>, SimError> {
    let mut files = Vec::with_capacity(count);
    for index in 0..count {
        let path = format!("{}{index}", valid_path(rng)?);
        let content = if rng.below(3)? == 0 {
            None
        } else {
            Some(format!("{}{}", hostile_body(rng)?, valid_path(rng)?))
        };
        files.push((path, content));
    }
    Ok(files)
}

/// The borrowed view of a brief's files that the prompt builders take.
fn views_of(files: &[(String, Option<String>)]) -> Vec<(&str, Option<&str>)> {
    files
        .iter()
        .map(|file| (file.0.as_str(), file.1.as_deref()))
        .collect()
}

/// double-clicker and monkey: for 256 seeds a brief of one to eight files, each existing
/// or new, in a seeded order. Every file's user turn starts with the one shared context
/// and adds exactly its own "write this file now" line; the context holds the files in
/// path-byte order whatever the brief's order, is byte-identical when built twice or
/// from a shuffled copy, and ends with the goal. Catches a prefix that moves with the
/// brief's order, a context that differs per file (so the provider's cache never hits),
/// or a per-request detail that leaks above the last line.
#[test]
fn sim_every_request_of_a_brief_shares_one_byte_stable_prefix() -> TestResult {
    for seed in 0..CACHE_SEEDS {
        let mut rng = Seeded::from_seed(seed);
        let count = usize::try_from(rng.below(8)?.saturating_add(1))?;
        let files = brief_files(&mut rng, count)?;
        let goal = format!("goal {seed} {}", hostile_body(&mut rng)?);
        let views = views_of(&files);
        let shared = shared_context(&goal, &views).ok_or("a small brief shares its context")?;
        assert_eq!(
            shared_context(&goal, &views).as_deref(),
            Some(shared.as_str()),
            "seed {seed}: building twice is byte-identical"
        );
        let mixed = shuffled(&mut rng, &views)?;
        assert_eq!(
            shared_context(&goal, &mixed).as_deref(),
            Some(shared.as_str()),
            "seed {seed}: a shuffled brief shares the same bytes"
        );
        assert!(
            shared.ends_with(&format!("\nGoal:\n{goal}\n")),
            "seed {seed}: the goal closes the context"
        );
        let mut sorted: Vec<&str> = views.iter().map(|view| view.0).collect();
        sorted.sort_unstable();
        let mut floor = 0_usize;
        for path in sorted {
            let at = shared
                .find(&format!("\n=== {path} "))
                .ok_or("every file appears in the context")?;
            assert!(
                at >= floor,
                "seed {seed}: {path} comes after the path before it"
            );
            floor = at.saturating_add(1);
        }
        for view in &views {
            let (path, current) = *view;
            let turn = user_turn(Some(&shared), &goal, path, current);
            let rest = turn
                .strip_prefix(shared.as_str())
                .ok_or("a user turn starts with the shared context")?;
            assert_eq!(
                rest,
                format!("\nWrite this file now: {path}"),
                "seed {seed}: only the last line names the file"
            );
        }
    }
    Ok(())
}

/// A seeded 64-character hex partition that begins with the digit at `lead`.
fn hex_partition(rng: &mut Seeded, lead: usize) -> Result<String, SimError> {
    /// The sixteen hex digits a partition is written in.
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut text = String::with_capacity(64);
    for place in 0..64_usize {
        let pick = if place == 0 { lead } else { rng.index(16)? };
        text.push(char::from(
            *DIGITS.get(pick).ok_or(SimError::Missing("hex digit"))?,
        ));
    }
    Ok(text)
}

/// neighbour: two tenants whose partitions differ (even in one leading digit) get system
/// lines that differ, both begin with the same standing instruction and differ only after
/// it, and one partition twice is byte-identical. Catches a prefix a provider's cache
/// would serve from one tenant's context to another's, and a line that moves with the clock.
#[test]
fn sim_two_tenants_never_share_a_prompt_prefix() -> TestResult {
    for seed in 0..CACHE_SEEDS {
        let mut rng = Seeded::from_seed(seed);
        let lead = rng.index(16)?;
        let other = if lead == 15 {
            0
        } else {
            lead.saturating_add(1)
        };
        let first = hex_partition(&mut rng, lead)?;
        let second = hex_partition(&mut rng, other)?;
        let route = openai_route()?;
        let (one, two) = (system_line(&route, &first), system_line(&route, &second));
        assert_ne!(one, two, "seed {seed}: two tenants, two system lines");
        let (head_one, head_two) = (one.strip_prefix(SYSTEM), two.strip_prefix(SYSTEM));
        assert_eq!(
            head_one,
            Some(format!("\nCache partition: {first}").as_str()),
            "seed {seed}: the first tenant's line is the instruction then its partition"
        );
        assert_ne!(
            head_one, head_two,
            "seed {seed}: the lines differ only after the instruction"
        );
        assert!(
            head_two.is_some(),
            "seed {seed}: the second begins with the instruction"
        );
        assert_eq!(
            system_line(&route, &first),
            one,
            "seed {seed}: one partition twice is identical"
        );
    }
    Ok(())
}

/// giant-and-empty: a brief whose context lands a seeded 1 to 64 bytes below, exactly at
/// and a seeded 1 to 64 bytes above the bound. The context is shared exactly when it fits;
/// when it is not, each file's prompt holds its own content and no other file's. Catches
/// an off-by-one at the bound, a context sent whole past it, and a fallback that still
/// leaks the neighbours' files.
#[test]
fn sim_a_context_past_its_bound_falls_back_to_one_file() -> TestResult {
    for seed in 0..CACHE_SEEDS {
        let mut rng = Seeded::from_seed(seed);
        let count = usize::try_from(rng.below(3)?.saturating_add(2))?;
        let goal = format!("goal {seed}");
        let paths: Vec<String> = (0..count)
            .map(|index| format!("dir{index}/file{index}.rs"))
            .collect();
        let markers: Vec<String> = (0..count)
            .map(|index| format!("marker-{seed}-{index}\n"))
            .collect();
        let probe_contents: Vec<&str> = markers
            .iter()
            .enumerate()
            .map(|(index, marker)| if index == 0 { "\n" } else { marker.as_str() })
            .collect();
        let probe_views: Vec<(&str, Option<&str>)> = paths
            .iter()
            .zip(&probe_contents)
            .map(|(path, content)| (path.as_str(), Some(*content)))
            .collect();
        let probe = shared_context(&goal, &probe_views)
            .ok_or("the probe fits")?
            .len();
        let delta = usize::try_from(rng.below(64)?.saturating_add(1))?;
        let target = match rng.below(3)? {
            0 => MAX_CONTEXT_BYTES.saturating_sub(delta),
            1 => MAX_CONTEXT_BYTES,
            _ => MAX_CONTEXT_BYTES.saturating_add(delta),
        };
        let pad = target.saturating_sub(probe).saturating_add(1);
        let big = format!("{}\n", "a".repeat(pad.saturating_sub(1)));
        let contents: Vec<&str> = markers
            .iter()
            .enumerate()
            .map(|(index, marker)| {
                if index == 0 {
                    big.as_str()
                } else {
                    marker.as_str()
                }
            })
            .collect();
        let views: Vec<(&str, Option<&str>)> = paths
            .iter()
            .zip(&contents)
            .map(|(path, content)| (path.as_str(), Some(*content)))
            .collect();
        let shared = shared_context(&goal, &views);
        assert_eq!(
            shared.is_some(),
            target <= MAX_CONTEXT_BYTES,
            "seed {seed}: shared exactly when {target} fits {MAX_CONTEXT_BYTES}"
        );
        if let Some(text) = shared.as_deref() {
            assert_eq!(
                text.len(),
                target,
                "seed {seed}: the bound is measured in bytes"
            );
            continue;
        }
        for (index, view) in views.iter().enumerate() {
            let (path, current) = *view;
            let turn = user_turn(None, &goal, path, current);
            assert!(
                turn.contains(contents.get(index).copied().ok_or("content")?),
                "seed {seed}: file {index} carries its own content"
            );
            for (other, marker) in markers.iter().enumerate().filter(|entry| entry.0 != index) {
                assert!(
                    !turn.contains(marker.as_str()),
                    "seed {seed}: file {index} never carries file {other}'s content"
                );
            }
            if index != 0 {
                assert!(
                    !turn.contains(big.as_str()),
                    "seed {seed}: file {index} never carries the big file"
                );
            }
        }
    }
    Ok(())
}

/// A client over a route whose request bound is `bound` bytes (the default when
/// `None`). Its pause refuses every wait, and the simulation only asks it `fits`,
/// so it never sends.
fn bounded_client(bound: Option<usize>) -> Result<ModelClient, ModelError> {
    Ok(ModelClient::new(
        sim_config(bound)?,
        Clock::virtual_at(Duration::ZERO),
        |_wait| false,
    ))
}

/// escaper (RFC 8259 section 7: a control byte is written as a six-byte `\u` escape):
/// for 64 seeds a two-file brief whose first file holds a seeded 180,000 to 260,000
/// control bytes. Its raw context fits MAX_CONTEXT_BYTES, but its encoded request passes
/// the default 1 MiB route bound, so the brief is not shared there and is shared on a
/// 16 MiB route. A small plain brief is shared on the default route and not on a 1 KiB
/// one, and one file whose request does not fit keeps the whole brief on one-file
/// prompts. Catches a raw-byte check that sends a context the client then refuses as
/// BodyTooLarge, which fails a brief the one-file prompts would have written, and a
/// check that shares the context when only some files' requests fit.
#[test]
fn sim_a_context_past_the_request_bound_falls_back_to_one_file() -> TestResult {
    let default_route = bounded_client(None)?;
    let wide_route = bounded_client(Some(16_777_216))?;
    let narrow_route = bounded_client(Some(1024))?;
    let template_route = openai_route()?;
    let paths = ["a/escaped.txt", "b/plain.txt"];
    for seed in 0..SEEDS {
        let mut rng = Seeded::from_seed(seed);
        let system = system_line(&template_route, &format!("{seed:064x}"));
        let count = usize::try_from(rng.below(80_001)?.saturating_add(180_000))?;
        let escaped = "\u{1}".repeat(count);
        let views = [
            (paths[0], Some(escaped.as_str())),
            (paths[1], Some("plain\n")),
        ];
        let raw = shared_context("goal", &views).ok_or("the raw context fits its byte bound")?;
        assert!(
            fitting_context(Some(raw.clone()), &system, &paths, |sent| default_route
                .fits(sent))
            .is_none(),
            "seed {seed}: {count} escaped bytes pass the 1 MiB request bound"
        );
        assert_eq!(
            fitting_context(Some(raw.clone()), &system, &paths, |sent| wide_route
                .fits(sent)),
            Some(raw),
            "seed {seed}: a 16 MiB route shares the same context"
        );
        let filler = "p".repeat(2048);
        let plain = [(paths[1], Some(filler.as_str()))];
        let small = shared_context(&format!("goal {seed}"), &plain).ok_or("a small brief fits")?;
        assert_eq!(
            fitting_context(Some(small.clone()), &system, &paths[1..], |sent| {
                default_route.fits(sent)
            })
            .as_deref(),
            Some(small.as_str()),
            "seed {seed}: a small brief is shared on the default route"
        );
        assert!(
            fitting_context(Some(small), &system, &paths[1..], |sent| narrow_route
                .fits(sent))
            .is_none(),
            "seed {seed}: a 1 KiB route refuses a 2 KiB context"
        );
        assert!(
            fitting_context(None, &system, &paths, |_sent| true).is_none(),
            "seed {seed}: no context stays no context"
        );
        let context = format!("context {seed}");
        let refused = Message::new(Role::User, user_turn(Some(&context), "", paths[1], None));
        assert!(
            fitting_context(Some(context.clone()), &system, &paths, |sent| !sent
                .contains(&refused))
            .is_none(),
            "seed {seed}: one file whose request does not fit keeps every file on its own prompt"
        );
        assert_eq!(prefix_label(Some(context.as_str())), "shared");
        assert_eq!(prefix_label(None), "own");
    }
    Ok(())
}

/// giant-and-empty (larry#396): for 64 seeds an empty reply, one of only whitespace and an
/// empty fence is refused `EmptyReply`, and a reply with one visible character keeps it with
/// a final newline. Catches a step that writes a cut reply as an empty file.
#[test]
fn sim_an_empty_reply_is_never_written_as_the_file() -> TestResult {
    for seed in 0..SEEDS {
        let mut rng = Seeded::from_seed(seed);
        let pad = " \t\n".repeat(usize::try_from(rng.below(4)?)?);
        for empty in [String::new(), pad.clone(), format!("```\n{pad}```")] {
            assert!(
                matches!(
                    reply_file(Kind::OpenAiChat, &empty),
                    Err(AuthorFault::EmptyReply)
                ),
                "seed {seed}: {empty:?} is refused"
            );
        }
        assert_eq!(
            reply_file(Kind::OpenAiChat, &format!("{pad}x"))?,
            format!("{pad}x\n"),
            "seed {seed}: one visible character is a file"
        );
    }
    Ok(())
}
