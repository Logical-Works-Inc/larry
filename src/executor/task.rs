//! The task document `larry exec` parses from standard input (larry#85), and
//! `larry_submit_task` builds over MCP: an optional brief (`goal` and `files`)
//! asks the model to write those files before the acceptance commands run (larry#123).

use std::fmt;
use std::path::{Path, PathBuf};

use super::author::Brief;
use crate::model::is_route_id;

/// Maximum bytes accepted for one task document.
pub(crate) const MAX_TASK_BYTES: usize = 65_536;

/// Maximum acceptance commands in one task document.
const MAX_COMMANDS: usize = 16;

/// Maximum items in one acceptance command.
const MAX_ARGV: usize = 64;

/// Maximum bytes in one command item.
const MAX_ARG_BYTES: usize = 4096;

/// Maximum bytes in the base commit-ish.
const MAX_BASE_BYTES: usize = 256;

/// Maximum bytes in the repository path.
const MAX_REPO_BYTES: usize = 4096;

/// Maximum bytes in an idempotency key (larry#182 defect 3).
const MAX_KEY_BYTES: usize = 256;

/// Every key a task document may carry.
const KEYS: [&str; 8] = [
    "repo",
    "base",
    "acceptance",
    "model_commands",
    "goal",
    "files",
    "idempotency_key",
    "route",
];

/// One parsed `larry exec` task document.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct Task {
    /// Repository path the task runs in.
    repo: PathBuf,
    /// Base commit-ish the task runs from.
    base: String,
    /// Acceptance commands, each an argv array.
    acceptance: Vec<Vec<String>>,
    /// Whether the document asked for model commands.
    model_commands: bool,
    /// The goal and the files the model writes, when the document carries them.
    brief: Option<Brief>,
    /// The client's idempotency key: its statement that a retry of this document is
    /// the same request (larry#182 defect 3).
    idempotency_key: Option<String>,
    /// The id of the route the model step uses, when the document names one
    /// (larry#201); otherwise the tenant's default, else the environment's.
    route: Option<String>,
    /// The document bytes it was parsed from, kept to name the run.
    bytes: Vec<u8>,
}

impl Task {
    /// Repository path the task runs in.
    #[must_use]
    pub fn repo(&self) -> &Path {
        self.repo.as_path()
    }

    /// Base commit-ish the task runs from.
    #[must_use]
    pub fn base(&self) -> &str {
        self.base.as_str()
    }

    /// Acceptance commands, each an argv array.
    #[must_use]
    pub fn acceptance(&self) -> &[Vec<String>] {
        self.acceptance.as_slice()
    }

    /// Whether the document asked for model commands.
    #[must_use]
    pub const fn has_model_commands(&self) -> bool {
        self.model_commands
    }

    /// The goal and the files the model writes, when the document carries them.
    #[must_use]
    pub(crate) const fn brief(&self) -> Option<&Brief> {
        self.brief.as_ref()
    }

    /// The client's idempotency key, when it named one: the statement that a retry
    /// of this document is the same request, so a retry maps to the same run id and
    /// the runtime reports it as a duplicate (larry#182 defect 3).
    #[must_use]
    pub(crate) fn idempotency_key(&self) -> Option<&str> {
        self.idempotency_key.as_deref()
    }

    /// The id of the route the document names, when it names one.
    #[must_use]
    pub(crate) fn route(&self) -> Option<&str> {
        self.route.as_deref()
    }

    /// The document bytes it was parsed from.
    #[must_use]
    pub(crate) fn bytes(&self) -> &[u8] {
        self.bytes.as_slice()
    }

    /// Parses one task document from its raw bytes.
    pub fn parse(bytes: &[u8]) -> Result<Task, TaskError> {
        if bytes.len() > MAX_TASK_BYTES {
            return Err(TaskError::TooLarge { len: bytes.len() });
        }
        let text = std::str::from_utf8(bytes).map_err(TaskError::NotUtf8)?;
        let value = lgwks_std::json::from_str::<lgwks_std::json::Value>(text)
            .map_err(TaskError::Malformed)?;
        let object = value.as_object().ok_or(TaskError::NotAnObject)?;
        for (key, _) in object {
            if !KEYS.contains(&key) {
                return Err(TaskError::UnknownKey {
                    key: key.to_owned(),
                });
            }
        }
        if !object.contains_key("repo") {
            return Err(TaskError::MissingKey { key: "repo" });
        }
        if !object.contains_key("base") {
            return Err(TaskError::MissingKey { key: "base" });
        }
        if !object.contains_key("acceptance") {
            return Err(TaskError::MissingKey { key: "acceptance" });
        }
        let repo_text = object
            .get("repo")
            .and_then(|entry| entry.as_str())
            .ok_or(TaskError::WrongType { key: "repo" })?;
        let base_text = object
            .get("base")
            .and_then(|entry| entry.as_str())
            .ok_or(TaskError::WrongType { key: "base" })?;
        let acceptance_value = object
            .get("acceptance")
            .and_then(|entry| entry.as_array())
            .ok_or(TaskError::WrongType { key: "acceptance" })?;
        let mut acceptance: Vec<Vec<String>> = Vec::with_capacity(acceptance_value.len());
        for command in acceptance_value {
            let argv = command
                .as_array()
                .ok_or(TaskError::WrongType { key: "acceptance" })?;
            let mut owned_argv: Vec<String> = Vec::with_capacity(argv.len());
            for item in argv {
                let text_item = item
                    .as_str()
                    .ok_or(TaskError::WrongType { key: "acceptance" })?;
                owned_argv.push(text_item.to_owned());
            }
            acceptance.push(owned_argv);
        }
        if repo_text.is_empty() || repo_text.len() > MAX_REPO_BYTES || repo_text.contains('\0') {
            return Err(TaskError::BadRepo);
        }
        if base_text.is_empty()
            || base_text.len() > MAX_BASE_BYTES
            || base_text.starts_with('-')
            || base_text
                .chars()
                .any(|character| character.is_control() || character.is_whitespace())
        {
            return Err(TaskError::BadBase);
        }
        if acceptance.len() > MAX_COMMANDS {
            return Err(TaskError::TooManyCommands {
                count: acceptance.len(),
            });
        }
        for (index, argv) in acceptance.iter().enumerate() {
            if argv.is_empty() {
                return Err(TaskError::EmptyArgv { index });
            }
            if argv.len() > MAX_ARGV {
                return Err(TaskError::ArgvTooLong {
                    index,
                    len: argv.len(),
                });
            }
            for item in argv {
                if item.len() > MAX_ARG_BYTES {
                    return Err(TaskError::ArgTooLarge {
                        index,
                        len: item.len(),
                    });
                }
                if item.contains('\0') {
                    return Err(TaskError::ArgHasNul { index });
                }
            }
        }
        let brief = Brief::parse(object)?;
        let idempotency_key = match object.get("idempotency_key") {
            None => None,
            Some(entry) => Some(bounded_key(entry)?),
        };
        let route = match object.get("route") {
            None => None,
            Some(entry) => Some(route_id(entry)?),
        };
        Ok(Task {
            repo: PathBuf::from(repo_text),
            base: base_text.to_owned(),
            acceptance,
            model_commands: object.contains_key("model_commands"),
            brief,
            idempotency_key,
            route,
            bytes: bytes.to_vec(),
        })
    }

    /// The task a planner hands over (`larry mcp`, `larry eval`): `repo` at `HEAD`,
    /// the brief, and each acceptance command run as `sh -c <command>`. It renders
    /// the one task document and parses it, so every door shares one parser and
    /// one digest names the run (ADR 0008).
    pub(crate) fn shell(
        repo: &str,
        goal: &str,
        files: &[&str],
        acceptance: &[&str],
    ) -> Result<Task, TaskError> {
        Task::shell_via(repo, goal, files, acceptance, None)
    }

    /// [`Task::shell`] over the route `route` names, when it names one: the planner
    /// picks the route by id, and the document carries it under the run's digest.
    pub(crate) fn shell_via(
        repo: &str,
        goal: &str,
        files: &[&str],
        acceptance: &[&str],
        route: Option<&str>,
    ) -> Result<Task, TaskError> {
        let commands: Vec<lgwks_std::json::Value> = acceptance
            .iter()
            .map(|command| lgwks_std::json::json!(["sh", "-c", command]))
            .collect();
        let mut document = lgwks_std::json::json!({
            "repo": repo,
            "base": "HEAD",
            "goal": goal,
            "files": files,
            "acceptance": commands,
        });
        if let Some(named) = route
            && let Some(object) = document.as_object_mut()
        {
            object.insert(
                String::from("route"),
                lgwks_std::json::Value::String(named.to_owned().into()),
            );
        }
        Task::parse(document.to_string().as_bytes())
    }
}

/// The route id `entry` holds: a string of 1 to 64 bytes of `a-z`, `0-9`, `_`
/// and `-`. Anything else is refused at the door, so a hostile name never
/// reaches the store or an error message (larry#201).
fn route_id(entry: &lgwks_std::json::Value) -> Result<String, TaskError> {
    let text = entry
        .as_str()
        .ok_or(TaskError::WrongType { key: "route" })?;
    if !is_route_id(text) {
        return Err(TaskError::BadRoute);
    }
    Ok(text.to_owned())
}

/// The idempotency key `entry` holds: 1 to [`MAX_KEY_BYTES`] bytes with no control
/// byte. Empty is refused because it names no request, and a control byte because a
/// key is text a client reuses verbatim on its retry (larry#182 defect 3).
fn bounded_key(entry: &lgwks_std::json::Value) -> Result<String, TaskError> {
    let text = entry.as_str().ok_or(TaskError::WrongType {
        key: "idempotency_key",
    })?;
    if text.is_empty() || text.len() > MAX_KEY_BYTES || text.chars().any(char::is_control) {
        return Err(TaskError::BadIdempotencyKey);
    }
    Ok(text.to_owned())
}

/// Why a task document was refused.
#[derive(Debug)]
#[non_exhaustive]
pub enum TaskError {
    /// The document exceeded the byte limit.
    TooLarge {
        /// Document length in bytes.
        len: usize,
    },
    /// The document was not valid UTF-8.
    NotUtf8(std::str::Utf8Error),
    /// The document was not valid JSON.
    Malformed(lgwks_std::json::Error),
    /// The document was not a JSON object.
    NotAnObject,
    /// The document held an unrecognised key.
    UnknownKey {
        /// The unrecognised key.
        key: String,
    },
    /// A required key was absent.
    MissingKey {
        /// The absent key.
        key: &'static str,
    },
    /// A key held the wrong JSON type.
    WrongType {
        /// The key with the wrong type.
        key: &'static str,
    },
    /// The repository path was empty, oversize, or held a NUL byte.
    BadRepo,
    /// The base was empty, oversize, option-like, or held a control character or whitespace.
    BadBase,
    /// The document held too many acceptance commands.
    TooManyCommands {
        /// Number of acceptance commands.
        count: usize,
    },
    /// An acceptance command held no items.
    EmptyArgv {
        /// Index of the empty command.
        index: usize,
    },
    /// An acceptance command held too many items.
    ArgvTooLong {
        /// Index of the command.
        index: usize,
        /// Number of items in the command.
        len: usize,
    },
    /// A command item exceeded the byte limit.
    ArgTooLarge {
        /// Index of the command holding the item.
        index: usize,
        /// Item length in bytes.
        len: usize,
    },
    /// A command item held a NUL byte.
    ArgHasNul {
        /// Index of the command holding the item.
        index: usize,
    },
    /// The document carried `goal` without `files`, or `files` without `goal`.
    IncompleteBrief,
    /// The goal was blank, oversize, or held a NUL byte.
    BadGoal,
    /// The brief named no file or too many.
    FileCount {
        /// Number of files named.
        count: usize,
    },
    /// A file path was empty, oversize, not repo-relative, inside `.git`, or a repeat.
    BadFile {
        /// Index of the file.
        index: usize,
    },
    /// The idempotency key was empty, oversize, or held a control byte.
    BadIdempotencyKey,
    /// The route was not 1 to 64 bytes of `a-z`, `0-9`, `_` and `-`.
    BadRoute,
}

impl fmt::Display for TaskError {
    fn fmt(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::TooLarge { len } => write!(out, "task document is too large ({len} bytes)"),
            Self::NotUtf8(ref cause) => write!(out, "task document is not UTF-8: {cause}"),
            Self::Malformed(ref cause) => write!(out, "task document is not valid JSON: {cause}"),
            Self::NotAnObject => out.write_str("task document is not a JSON object"),
            Self::UnknownKey { ref key } => write!(out, "task document has unknown key {key}"),
            Self::MissingKey { key } => write!(out, "task document is missing key {key}"),
            Self::WrongType { key } => write!(out, "task document key {key} has the wrong type"),
            Self::BadRepo => out.write_str("task document repository path is invalid"),
            Self::BadBase => out.write_str("task document base is invalid"),
            Self::TooManyCommands { count } => {
                write!(out, "task document has too many commands ({count})")
            }
            Self::EmptyArgv { index } => write!(out, "task document command {index} is empty"),
            Self::ArgvTooLong { index, len } => {
                write!(
                    out,
                    "task document command {index} has too many items ({len})"
                )
            }
            Self::ArgTooLarge { index, len } => {
                write!(
                    out,
                    "task document command {index} has an item that is too large ({len} bytes)"
                )
            }
            Self::ArgHasNul { index } => {
                write!(
                    out,
                    "task document command {index} has an item holding a NUL byte"
                )
            }
            Self::IncompleteBrief => {
                out.write_str("task document needs both goal and files, or neither")
            }
            Self::BadGoal => out.write_str("task document goal is blank, too long or holds NUL"),
            Self::FileCount { count } => {
                write!(
                    out,
                    "task document names {count} files; 1 to 64 are allowed"
                )
            }
            Self::BadFile { index } => write!(
                out,
                "task document file {index} is not a distinct repo-relative path outside .git"
            ),
            Self::BadIdempotencyKey => write!(
                out,
                "task document idempotency_key is empty, over {MAX_KEY_BYTES} bytes, or holds a control byte"
            ),
            Self::BadRoute => out.write_str(
                "task document route is not a route id: 1 to 64 bytes of a-z, 0-9, _ and -",
            ),
        }
    }
}

impl std::error::Error for TaskError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match *self {
            Self::NotUtf8(ref cause) => Some(cause),
            Self::Malformed(ref cause) => Some(cause),
            _ => None,
        }
    }
}

/// Seeded simulation tests for the task parser.
#[cfg(test)]
mod tests {
    use super::{MAX_ARG_BYTES, MAX_BASE_BYTES, MAX_REPO_BYTES, MAX_TASK_BYTES, Task, TaskError};

    /// SplitMix64 generator driving every simulation case from one seed.
    struct SplitMix64 {
        /// Current generator state.
        state: u64,
    }

    impl SplitMix64 {
        /// Builds a generator from a seed.
        fn new(seed: u64) -> SplitMix64 {
            SplitMix64 { state: seed }
        }

        /// Draws the next 64-bit value.
        fn next_u64(&mut self) -> u64 {
            self.state = self.state.wrapping_add(0x9e37_79b9_7f4a_7c15);
            let mut mixed = self.state;
            mixed = (mixed ^ (mixed >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
            mixed = (mixed ^ (mixed >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
            mixed ^ (mixed >> 31)
        }

        /// Draws a value below the bound.
        fn next_bound(&mut self, bound: u64) -> Result<u64, Box<dyn std::error::Error>> {
            let drawn = self.next_u64();
            let product = u128::from(drawn)
                .checked_mul(u128::from(bound))
                .ok_or("seeded draw overflow")?;
            Ok(u64::try_from(product >> 64)?)
        }

        /// Draws a printable ASCII string of the given length.
        fn next_text(&mut self, length: usize) -> Result<String, Box<dyn std::error::Error>> {
            let mut text = String::with_capacity(length);
            let mut drawn = 0_usize;
            while drawn < length {
                let byte = u8::try_from(self.next_bound(95)?.saturating_add(32))?;
                text.push(char::from(byte));
                drawn = drawn.saturating_add(1);
            }
            Ok(text)
        }

        /// Draws an alphanumeric word of the given length: a base that every
        /// rule accepts (no leading `-`, no whitespace or control character).
        fn next_word(&mut self, length: usize) -> Result<String, Box<dyn std::error::Error>> {
            /// The alphabet a valid base is drawn from.
            const ALPHABET: &[u8] =
                b"abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789";
            let mut text = String::with_capacity(length);
            while text.len() < length {
                let index = usize::try_from(self.next_bound(u64::try_from(ALPHABET.len())?)?)?;
                let byte = ALPHABET
                    .get(index)
                    .ok_or("seeded draw outside the alphabet")?;
                text.push(char::from(*byte));
            }
            Ok(text)
        }
    }

    /// One random valid task: its bytes and every field it must read back.
    struct ValidTask {
        /// Rendered document bytes.
        bytes: Vec<u8>,
        /// Repository path it carries.
        repo: String,
        /// Base commit-ish it carries.
        base: String,
        /// Acceptance commands it carries.
        acceptance: Vec<Vec<String>>,
        /// Whether it carries `model_commands`.
        with_model: bool,
    }

    /// Builds one random valid task for a seed.
    fn valid_task(generator: &mut SplitMix64) -> Result<ValidTask, Box<dyn std::error::Error>> {
        let repo = generator.next_text(8)?;
        let base = generator.next_word(8)?;
        let command_count = usize::try_from(generator.next_bound(17)?)?;
        let mut acceptance: Vec<Vec<String>> = Vec::with_capacity(command_count);
        let mut command_index = 0_usize;
        while command_index < command_count {
            let item_count = usize::try_from(generator.next_bound(64)?.saturating_add(1))?;
            let mut argv: Vec<String> = Vec::with_capacity(item_count);
            let mut item_index = 0_usize;
            while item_index < item_count {
                argv.push(generator.next_text(6)?);
                item_index = item_index.saturating_add(1);
            }
            acceptance.push(argv);
            command_index = command_index.saturating_add(1);
        }
        let with_model = generator.next_bound(2)? == 1;
        let mut rendered = lgwks_std::json::json!({
            "repo": repo.as_str(),
            "base": base.as_str(),
            "acceptance": acceptance.clone(),
        });
        if with_model && let Some(object) = rendered.as_object_mut() {
            object.insert("model_commands".to_owned(), lgwks_std::json::json!(true));
        }
        let bytes = rendered.to_string().into_bytes();
        Ok(ValidTask {
            bytes,
            repo,
            base,
            acceptance,
            with_model,
        })
    }

    /// Renders a minimal valid document given its parts.
    fn rendered(repo: &str, base: &str, acceptance: lgwks_std::json::Value) -> Vec<u8> {
        lgwks_std::json::json!({"repo": repo, "base": base, "acceptance": acceptance})
            .to_string()
            .into_bytes()
    }

    /// Every seeded valid task roundtrips through the parser.
    #[test]
    fn sim_valid_tasks_roundtrip_for_every_seed() -> Result<(), Box<dyn std::error::Error>> {
        let mut seed = 0_u64;
        while seed < 64 {
            let mut generator = SplitMix64::new(seed);
            let valid = valid_task(&mut generator)?;
            let task = Task::parse(&valid.bytes)?;
            assert_eq!(task.repo(), std::path::Path::new(&valid.repo));
            assert_eq!(task.base(), valid.base.as_str());
            assert_eq!(task.acceptance(), valid.acceptance.as_slice());
            assert_eq!(task.has_model_commands(), valid.with_model);
            seed = seed.saturating_add(1);
        }
        Ok(())
    }

    /// Every seeded hostile document gets its exact error.
    #[test]
    fn sim_hostile_tasks_get_their_exact_error_for_every_seed()
    -> Result<(), Box<dyn std::error::Error>> {
        let mut seed = 0_u64;
        while seed < 64 {
            let mut generator = SplitMix64::new(seed);
            let tag = format!("x{}", generator.next_word(4)?);
            let repo = generator.next_text(4)?;
            let base = generator.next_word(4)?;
            let oversize = vec![b'a'; MAX_TASK_BYTES.saturating_add(1)];
            assert!(matches!(
                Task::parse(&oversize),
                Err(TaskError::TooLarge { len })
                    if len == MAX_TASK_BYTES.saturating_add(1)
            ));
            assert!(matches!(
                Task::parse(&[0xff, 0xfe]),
                Err(TaskError::NotUtf8(_))
            ));
            assert!(matches!(Task::parse(b"[[[["), Err(TaskError::Malformed(_))));
            assert!(matches!(Task::parse(b"[1,2]"), Err(TaskError::NotAnObject)));
            let mut unknown =
                lgwks_std::json::json!({"repo": repo, "base": base, "acceptance": []});
            if let Some(object) = unknown.as_object_mut() {
                object.insert(tag.clone(), lgwks_std::json::json!(1));
            }
            assert!(matches!(
                Task::parse(unknown.to_string().as_bytes()),
                Err(TaskError::UnknownKey { ref key }) if key == &tag
            ));
            let missing_repo = lgwks_std::json::json!({"base": base, "acceptance": []});
            assert!(matches!(
                Task::parse(missing_repo.to_string().as_bytes()),
                Err(TaskError::MissingKey { key: "repo" })
            ));
            let missing_base = lgwks_std::json::json!({"repo": repo, "acceptance": []});
            assert!(matches!(
                Task::parse(missing_base.to_string().as_bytes()),
                Err(TaskError::MissingKey { key: "base" })
            ));
            let missing_commands = lgwks_std::json::json!({"repo": repo, "base": base});
            assert!(matches!(
                Task::parse(missing_commands.to_string().as_bytes()),
                Err(TaskError::MissingKey { key: "acceptance" })
            ));
            let wrong_repo = lgwks_std::json::json!({"repo": 7, "base": base, "acceptance": []});
            assert!(matches!(
                Task::parse(wrong_repo.to_string().as_bytes()),
                Err(TaskError::WrongType { key: "repo" })
            ));
            let wrong_base = lgwks_std::json::json!({"repo": repo, "base": true, "acceptance": []});
            assert!(matches!(
                Task::parse(wrong_base.to_string().as_bytes()),
                Err(TaskError::WrongType { key: "base" })
            ));
            let wrong_commands =
                lgwks_std::json::json!({"repo": repo, "base": base, "acceptance": "run"});
            assert!(matches!(
                Task::parse(wrong_commands.to_string().as_bytes()),
                Err(TaskError::WrongType { key: "acceptance" })
            ));
            let wrong_command =
                lgwks_std::json::json!({"repo": repo, "base": base, "acceptance": ["run"]});
            assert!(matches!(
                Task::parse(wrong_command.to_string().as_bytes()),
                Err(TaskError::WrongType { key: "acceptance" })
            ));
            let wrong_item =
                lgwks_std::json::json!({"repo": repo, "base": base, "acceptance": [[1]]});
            assert!(matches!(
                Task::parse(wrong_item.to_string().as_bytes()),
                Err(TaskError::WrongType { key: "acceptance" })
            ));
            assert!(matches!(
                Task::parse(&rendered("", &base, lgwks_std::json::json!([]))),
                Err(TaskError::BadRepo)
            ));
            let big_repo = "r".repeat(MAX_REPO_BYTES.saturating_add(1));
            assert!(matches!(
                Task::parse(&rendered(&big_repo, &base, lgwks_std::json::json!([]))),
                Err(TaskError::BadRepo)
            ));
            assert!(matches!(
                Task::parse(&rendered("a\0b", &base, lgwks_std::json::json!([]))),
                Err(TaskError::BadRepo)
            ));
            assert!(matches!(
                Task::parse(&rendered(&repo, "", lgwks_std::json::json!([]))),
                Err(TaskError::BadBase)
            ));
            let big_base = "b".repeat(MAX_BASE_BYTES.saturating_add(1));
            assert!(matches!(
                Task::parse(&rendered(&repo, &big_base, lgwks_std::json::json!([]))),
                Err(TaskError::BadBase)
            ));
            assert!(matches!(
                Task::parse(&rendered(&repo, "-main", lgwks_std::json::json!([]))),
                Err(TaskError::BadBase)
            ));
            assert!(matches!(
                Task::parse(&rendered(&repo, "a b", lgwks_std::json::json!([]))),
                Err(TaskError::BadBase)
            ));
            assert!(matches!(
                Task::parse(&rendered(&repo, "a\tb", lgwks_std::json::json!([]))),
                Err(TaskError::BadBase)
            ));
            let mut many_commands = Vec::new();
            let mut command_index = 0_usize;
            while command_index < 17 {
                many_commands.push(lgwks_std::json::json!(["run"]));
                command_index = command_index.saturating_add(1);
            }
            let many_commands = lgwks_std::json::Value::Array(many_commands);
            assert!(matches!(
                Task::parse(&rendered(&repo, &base, many_commands)),
                Err(TaskError::TooManyCommands { count: 17 })
            ));
            assert!(matches!(
                Task::parse(&rendered(&repo, &base, lgwks_std::json::json!([[]]))),
                Err(TaskError::EmptyArgv { index: 0 })
            ));
            let mut long_command = Vec::new();
            let mut item_index = 0_usize;
            while item_index < 65 {
                long_command.push(lgwks_std::json::json!("run"));
                item_index = item_index.saturating_add(1);
            }
            let long_argv =
                lgwks_std::json::Value::Array(vec![lgwks_std::json::Value::Array(long_command)]);
            assert!(matches!(
                Task::parse(&rendered(&repo, &base, long_argv)),
                Err(TaskError::ArgvTooLong { index: 0, len: 65 })
            ));
            let big_item = "x".repeat(MAX_ARG_BYTES.saturating_add(1));
            let big_argv = lgwks_std::json::json!([[big_item]]);
            assert!(matches!(
                Task::parse(&rendered(&repo, &base, big_argv)),
                Err(TaskError::ArgTooLarge { index: 0, len }) if len == MAX_ARG_BYTES.saturating_add(1)
            ));
            assert!(matches!(
                Task::parse(&rendered(&repo, &base, lgwks_std::json::json!([["a\0b"]]))),
                Err(TaskError::ArgHasNul { index: 0 })
            ));
            seed = seed.saturating_add(1);
        }
        Ok(())
    }
}
