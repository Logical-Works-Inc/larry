//! The temp-path rule (larry#267, larry#293): in test code a temp directory may be named with the
//! process id only inside the one guard that empties the directory before use and removes it on
//! drop. A fixture named `<label>-<pid>` that is never removed lands a later run in a dead run's
//! tree when the system reuses that id: larry#243 measured 3,566 such directories in `$TMPDIR`,
//! and larry#267 names six fixtures whose stale contents failed the tests that followed them.
//!
//! The rule reads the source's tokens, not its bytes on a line (larry#293): a `let` binding of the
//! process id, a `use` alias of `std::process`, `std::env`, `id` or `temp_dir`, a shadowing `let`
//! and the code passed to a macro are resolved by the name they bind, so only a string literal's
//! own content is data. The directory walk never follows a symlink (CWE-59).
//!
//! Source: The Rust Reference on path expressions and use declarations, POSIX `mkstemp(3)` (a
//! temporary name must be unique and owned by its maker) and CWE-377 (insecure temporary file).

use std::collections::BTreeMap;
use std::io;
use std::path::{Path, PathBuf};

use super::tokens::{Lexeme, Token, lex, macro_mask};

/// The one guard of the crate's tests and the one guard of the integration tests. Each empties its
/// directory before use and removes it on drop (`executor::sim_tree::scratch`, `TempDir` behind
/// `support/temp_dir.rs`), so the process id there names a directory its owner replaces and then
/// removes, never a leftover. Every other temp path in test code reaches its directory through one
/// of them.
const GUARDS: [&str; 2] = ["src/executor/sim_tree.rs", "tests/support/temp_dir.rs"];

/// The largest source file the rule reads, in bytes.
const MAX_SOURCE_BYTES: usize = 2 * 1024 * 1024;

/// The directories the rule reads, in order.
const ROOTS: [&str; 2] = ["src", "tests"];

/// The deepest nesting the walk follows, so a pathological tree ends in a refusal, never a stack
/// overflow.
const MAX_DEPTH: usize = 64;

/// Every refusal the rule raises over the repository at `root`, in path and line order.
pub(crate) fn errors(root: &Path) -> Vec<String> {
    let mut errors = Vec::new();
    let mut sources = Vec::new();
    for directory in ROOTS {
        match collect(&root.join(directory), directory, 0, &mut sources) {
            Ok(()) => {}
            // A fixture tree holds the documents alone; no source directory is not a refusal.
            Err(cause) if cause.kind() == io::ErrorKind::NotFound => {}
            Err(cause) => errors.push(format!("unreadable source tree: {directory}: {cause}")),
        }
    }
    sources.sort();
    for relative in sources {
        if GUARDS.contains(&relative.as_str()) {
            continue;
        }
        collect_errors(root, &relative, &mut errors);
    }
    errors
}

/// The refusals one source file raises, pushed onto `errors`.
fn collect_errors(root: &Path, relative: &str, errors: &mut Vec<String>) {
    let bytes = match std::fs::read(root.join(relative)) {
        Ok(bytes) => bytes,
        Err(cause) => {
            errors.push(format!("unreadable source file: {relative}: {cause}"));
            return;
        }
    };
    if bytes.len() > MAX_SOURCE_BYTES {
        errors.push(format!("source file over 2 MiB: {relative}"));
        return;
    }
    let text = String::from_utf8_lossy(&bytes);
    if is_test_code(relative, &text) {
        errors.extend(pid_temp_paths(relative, &text));
    }
}

/// Every `.rs` file beneath `path`, as a root-relative path with `/`. A symlink is never followed,
/// whatever it points at, and nesting past [`MAX_DEPTH`] is refused by name.
fn collect(path: &Path, relative: &str, depth: usize, found: &mut Vec<String>) -> io::Result<()> {
    if depth > MAX_DEPTH {
        return Err(io::Error::other(format!(
            "source tree deeper than {MAX_DEPTH}"
        )));
    }
    let mut entries: Vec<(PathBuf, bool, bool)> = Vec::new();
    for entry in std::fs::read_dir(path)? {
        let entry = entry?;
        let file_type = entry.file_type()?;
        entries.push((entry.path(), file_type.is_dir(), file_type.is_symlink()));
    }
    entries.sort();
    for (entry, is_dir, is_symlink) in entries {
        let Some(name) = entry.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        if is_symlink {
            continue;
        }
        let child = format!("{relative}/{name}");
        if is_dir {
            collect(&entry, &child, depth.saturating_add(1), found)?;
        } else if name.ends_with(".rs") {
            found.push(child);
        }
    }
    Ok(())
}

/// Whether `relative` is test code: every file under `tests/`, and an in-crate file that is a
/// simulation module, a tests module or holds one behind `#[cfg(test)]`.
fn is_test_code(relative: &str, text: &str) -> bool {
    if relative.starts_with("tests/") {
        return true;
    }
    let Some(stem) = Path::new(relative)
        .file_stem()
        .and_then(|stem| stem.to_str())
    else {
        return text.contains("#[cfg(test)]");
    };
    stem.starts_with("sim_")
        || stem.ends_with("_tests")
        || stem == "tests"
        || text.contains("#[cfg(test)]")
}

/// Every line of `relative` whose code names a temp directory with a process id.
fn pid_temp_paths(relative: &str, text: &str) -> Vec<String> {
    let lexemes = lex(text);
    let aliases = aliases(&lexemes);
    let in_macro = macro_mask(&lexemes);
    let mut scopes: Vec<BTreeMap<String, Binding>> = vec![BTreeMap::new()];
    let mut errors = Vec::new();
    let mut start = 0_usize;
    for (index, lexeme) in lexemes.iter().enumerate() {
        if !matches!(lexeme.token, Token::Punct(';' | '{' | '}')) {
            continue;
        }
        let opens = matches!(lexeme.token, Token::Punct('{'));
        let closes = matches!(lexeme.token, Token::Punct('}'));
        if let Some(statement) = lexemes.get(start..=index) {
            if let Some(line) = temp_line(statement, &aliases)
                && names_pid(statement, start, &aliases, &in_macro, &scopes)
            {
                errors.push(format!(
                    "a process id names a temp directory outside the guard: {relative}:{line}"
                ));
            }
            bind_let(statement, &aliases, &mut scopes);
            if !opens && !closes {
                balance(statement, &mut scopes);
            }
        }
        if closes {
            pop_scope(&mut scopes);
        }
        if opens {
            scopes.push(BTreeMap::new());
        }
        start = index.saturating_add(1);
    }
    errors
}

/// Applies the scope changes of the braces a `;` boundary keeps inside one statement; they always
/// balance, so no binding outlives the block that held it.
fn balance(statement: &[Lexeme<'_>], scopes: &mut Vec<BTreeMap<String, Binding>>) {
    for lexeme in statement {
        match lexeme.token {
            Token::Punct('{') => scopes.push(BTreeMap::new()),
            Token::Punct('}') => pop_scope(scopes),
            _ => {}
        }
    }
}

/// Drops the innermost scope, keeping the module scope.
fn pop_scope(scopes: &mut Vec<BTreeMap<String, Binding>>) {
    if scopes.len() > 1 {
        drop(scopes.pop());
    }
}

/// What a local name holds for the rule.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Binding {
    /// The process id, from `std::process::id` or an alias of it.
    Pid,
    /// Anything else, including a name that shadows a process id.
    Other,
}

/// What a `use` declaration makes a local name mean.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Alias {
    /// The `std::process` module.
    Process,
    /// The `std::env` module.
    Env,
    /// The `std::process::id` call.
    Pid,
    /// The `std::env::temp_dir` call.
    Temp,
}

/// The local names a file's `use` declarations bind, by the meaning each has.
type Aliases = BTreeMap<String, Alias>;

/// One path a `use` declaration reaches and the local name it binds.
struct UseItem {
    /// The full path, segment by segment, without a leading `::`.
    path: Vec<String>,
    /// The `as` name the declaration gives it, when one is given.
    alias: Option<String>,
}

/// The names the file's `use` declarations bind.
fn aliases(lexemes: &[Lexeme<'_>]) -> Aliases {
    let mut names = Aliases::new();
    let mut at = 0_usize;
    for _ in 0..lexemes.len().saturating_add(1) {
        let Some(lexeme) = lexemes.get(at) else {
            break;
        };
        if matches!(lexeme.token, Token::Name(name) if name == "use") {
            let mut cursor = at.saturating_add(1);
            let mut items = Vec::new();
            parse_use_tree(lexemes, &mut cursor, &[], &mut items);
            for item in &items {
                if let Some((name, alias)) = alias_of(item) {
                    names.insert(name, alias);
                }
            }
            at = cursor.max(at.saturating_add(1));
            continue;
        }
        at = at.saturating_add(1);
    }
    names
}

/// Parses one `use` tree at `at`, extending `base` with the segments it names.
fn parse_use_tree(
    lexemes: &[Lexeme<'_>],
    at: &mut usize,
    base: &[String],
    items: &mut Vec<UseItem>,
) {
    let mut path = base.to_vec();
    for _ in 0..lexemes.len().saturating_add(1) {
        let Some(lexeme) = lexemes.get(*at) else {
            break;
        };
        match lexeme.token {
            Token::Name("as") => {
                *at = at.saturating_add(1);
                if let Some(Token::Name(alias)) = lexemes.get(*at).map(|next| next.token) {
                    items.push(UseItem {
                        path,
                        alias: Some(String::from(alias)),
                    });
                    *at = at.saturating_add(1);
                }
                return;
            }
            Token::Name(name) => {
                path.push(String::from(name));
                *at = at.saturating_add(1);
            }
            Token::PathSep => {
                *at = at.saturating_add(1);
                match lexemes.get(*at).map(|next| next.token) {
                    Some(Token::Punct('{')) => {
                        *at = at.saturating_add(1);
                        parse_use_group(lexemes, at, &path, items);
                        return;
                    }
                    Some(Token::Punct('*')) => {
                        *at = at.saturating_add(1);
                        return;
                    }
                    _ => {}
                }
            }
            Token::Punct('{') => {
                *at = at.saturating_add(1);
                parse_use_group(lexemes, at, &path, items);
                return;
            }
            Token::Text(_) | Token::Literal | Token::Punct(_) => {
                if !path.is_empty() {
                    items.push(UseItem { path, alias: None });
                }
                return;
            }
        }
    }
    if !path.is_empty() {
        items.push(UseItem { path, alias: None });
    }
}

/// Parses the `{...}` group of a `use` tree at `at`.
fn parse_use_group(
    lexemes: &[Lexeme<'_>],
    at: &mut usize,
    base: &[String],
    items: &mut Vec<UseItem>,
) {
    for _ in 0..lexemes.len().saturating_add(1) {
        let Some(lexeme) = lexemes.get(*at) else {
            return;
        };
        if matches!(lexeme.token, Token::Punct('}')) {
            *at = at.saturating_add(1);
            return;
        }
        if matches!(lexeme.token, Token::Punct(',')) {
            *at = at.saturating_add(1);
            continue;
        }
        parse_use_tree(lexemes, at, base, items);
    }
}

/// The local name `item` binds and what it means, when the path is the process id, the temp
/// directory or the module that holds one of them.
fn alias_of(item: &UseItem) -> Option<(String, Alias)> {
    let local = match item.alias.clone() {
        Some(name) => name,
        None => String::from(item.path.last()?.as_str()),
    };
    let segments: Vec<&str> = item.path.iter().map(String::as_str).collect();
    let alias = match *segments.as_slice() {
        ["std", "process", "id"] => Alias::Pid,
        ["std", "env", "temp_dir"] => Alias::Temp,
        ["std", "process"] => Alias::Process,
        ["std", "env"] => Alias::Env,
        _ => return None,
    };
    Some((local, alias))
}

/// The meaning of the call whose path starts at `at`, when it is the process id or the temp
/// directory and is followed by an argument list.
fn call_at(lexemes: &[Lexeme<'_>], at: usize, aliases: &Aliases) -> Option<Alias> {
    let Some(Token::Name(first)) = lexemes.get(at).map(|lexeme| lexeme.token) else {
        return None;
    };
    let mut segments: Vec<&str> = vec![first];
    let mut cursor = at.saturating_add(1);
    for _ in 0..lexemes.len().saturating_add(1) {
        if !matches!(
            lexemes.get(cursor).map(|lexeme| lexeme.token),
            Some(Token::PathSep)
        ) {
            break;
        }
        let Some(Token::Name(name)) = lexemes.get(cursor.saturating_add(1)).map(|next| next.token)
        else {
            break;
        };
        segments.push(name);
        cursor = cursor.saturating_add(2);
    }
    if !matches!(
        lexemes.get(cursor).map(|lexeme| lexeme.token),
        Some(Token::Punct('('))
    ) {
        return None;
    }
    match *segments.as_slice() {
        ["std", "process", "id"] => Some(Alias::Pid),
        ["std", "env", "temp_dir"] => Some(Alias::Temp),
        [module, "id"] if aliases.get(module) == Some(&Alias::Process) => Some(Alias::Pid),
        [module, "temp_dir"] if aliases.get(module) == Some(&Alias::Env) => Some(Alias::Temp),
        [single] => match aliases.get(single) {
            Some(&Alias::Pid) => Some(Alias::Pid),
            Some(&Alias::Temp) => Some(Alias::Temp),
            Some(&Alias::Process) | Some(&Alias::Env) | None => None,
        },
        _ => None,
    }
}

/// The line of the first temp-directory call in `statement`.
fn temp_line(statement: &[Lexeme<'_>], aliases: &Aliases) -> Option<usize> {
    for (offset, lexeme) in statement.iter().enumerate() {
        if !matches!(lexeme.token, Token::Name(_)) {
            continue;
        }
        if call_at(statement, offset, aliases) == Some(Alias::Temp) {
            return Some(lexeme.line);
        }
    }
    None
}

/// Whether `statement`, which starts at absolute token `start`, names the process id as code or as
/// a binding, including a format macro's `{name}` capture.
fn names_pid(
    statement: &[Lexeme<'_>],
    start: usize,
    aliases: &Aliases,
    in_macro: &[bool],
    scopes: &[BTreeMap<String, Binding>],
) -> bool {
    for (offset, lexeme) in statement.iter().enumerate() {
        match lexeme.token {
            Token::Name(name) => {
                if call_at(statement, offset, aliases) == Some(Alias::Pid) {
                    return true;
                }
                if binding(scopes, name) == Some(Binding::Pid) {
                    return true;
                }
            }
            Token::Text(text)
                if in_macro.get(start.saturating_add(offset)) == Some(&true)
                    && captures(text)
                        .iter()
                        .any(|name| binding(scopes, name) == Some(Binding::Pid)) =>
            {
                return true;
            }
            _ => {}
        }
    }
    false
}

/// The innermost binding of `name`.
fn binding(scopes: &[BTreeMap<String, Binding>], name: &str) -> Option<Binding> {
    scopes
        .iter()
        .rev()
        .find_map(|scope| scope.get(name).copied())
}

/// Records the name a `let` statement binds, and whether it holds the process id, in the
/// innermost scope; a later `let` of the same name shadows this one.
fn bind_let(statement: &[Lexeme<'_>], aliases: &Aliases, scopes: &mut [BTreeMap<String, Binding>]) {
    if !matches!(statement.first().map(|first| first.token), Some(Token::Name(word)) if word == "let")
    {
        return;
    }
    let mut cursor = 1_usize;
    for _ in 0..statement.len().saturating_add(1) {
        if matches!(
            statement.get(cursor).map(|lexeme| lexeme.token),
            Some(Token::Name(word)) if word == "mut" || word == "ref"
        ) {
            cursor = cursor.saturating_add(1);
        } else {
            break;
        }
    }
    let Some(Token::Name(name)) = statement.get(cursor).map(|lexeme| lexeme.token) else {
        return;
    };
    match statement
        .get(cursor.saturating_add(1))
        .map(|lexeme| lexeme.token)
    {
        Some(Token::Punct('=' | ':' | ';')) => {}
        _ => return,
    }
    let assignment = statement
        .iter()
        .position(|lexeme| matches!(lexeme.token, Token::Punct('=')));
    let Some(rhs) = assignment.and_then(|at| statement.get(at.saturating_add(1)..)) else {
        return;
    };
    let holds = if rhs.iter().enumerate().any(|(offset, lexeme)| {
        matches!(lexeme.token, Token::Name(_)) && call_at(rhs, offset, aliases) == Some(Alias::Pid)
    }) {
        Binding::Pid
    } else {
        Binding::Other
    };
    if let Some(scope) = scopes.last_mut() {
        scope.insert(String::from(name), holds);
    }
}

/// The names a `format!`-style string captures: `{name}` and `{name:spec}`, never `{{` or `{}`.
fn captures(text: &str) -> Vec<String> {
    let mut names = Vec::new();
    let mut rest = text;
    for _ in 0..text.len().saturating_add(1) {
        let Some(open) = rest.find('{') else {
            break;
        };
        let Some(after) = rest.get(open.saturating_add(1)..) else {
            break;
        };
        if after.starts_with('{') {
            rest = match after.get(1..) {
                Some(next) => next,
                None => break,
            };
            continue;
        }
        let Some((inner, tail)) = after.split_once('}') else {
            break;
        };
        let name = match inner.split_once(':') {
            Some((before, _spec)) => before,
            None => inner,
        };
        let name = name.trim_end_matches('!');
        if is_simple_name(name) {
            names.push(String::from(name));
        }
        rest = tail;
    }
    names
}

/// Whether `name` is a plain identifier.
fn is_simple_name(name: &str) -> bool {
    let mut characters = name.chars();
    characters
        .next()
        .is_some_and(|first| first.is_alphabetic() || first == '_')
        && characters.all(|character| character.is_alphanumeric() || character == '_')
}
