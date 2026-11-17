//! The verdicts of the temp-path rule over one parsed source (larry#293, ADR 0058): which
//! functions reach a temp source, which name the process id, and which names are a pattern, a
//! method or a shadow rather than the id. Source: The Rust Reference on path expressions,
//! patterns and macros-by-example, and the tree-sitter Rust grammar's node kinds.

use std::collections::BTreeSet;
use std::path::Path;

use lgwks_ast::{AstNode, Parsed};

use super::uses::Uses;

/// The names a temp directory string literal carries after its leading path separator: the two
/// POSIX system temporary directory roots, read as source text and never as a host path.
const TEMP_NAMES: [&str; 2] = ["tmp", "var/tmp"];

/// Whether a string literal's content names a system temp directory: one separator, then a name
/// of [`TEMP_NAMES`].
fn starts_temp(content: &str) -> bool {
    match content.strip_prefix('/') {
        Some(rest) => TEMP_NAMES.iter().any(|name| rest.starts_with(name)),
        None => false,
    }
}

/// The variable `env!` and `var` name when the temp directory is read from the environment.
const TMPDIR: &str = "TMPDIR";

/// The node kinds that hold a name the rule can read, and never a body of their own.
const SIMPLE_NAMES: [&str; 4] = ["identifier", "crate", "super", "self"];

/// The node kinds whose `name` field is a definition, not a use of that name.
const DEFINITIONS: [&str; 11] = [
    "function_item",
    "struct_item",
    "enum_item",
    "union_item",
    "trait_item",
    "mod_item",
    "field_declaration",
    "const_item",
    "static_item",
    "type_item",
    "macro_definition",
];

/// Every refusal one test file's functions raise.
pub(super) fn verdicts(relative: &str, text: &str, tree: &Parsed) -> Vec<String> {
    let root = tree.root();
    let uses = Uses::of(&root);
    let mut errors = Vec::new();
    for function in root.dfs() {
        if function.kind() != "function_item" {
            continue;
        }
        let bound = bound_names(&function);
        let Some(temp) = first_temp(&function, &uses, &bound) else {
            continue;
        };
        let Some(pid) = first_pid(&function, &uses, &bound) else {
            continue;
        };
        errors.push(format!(
            "a process id names a temp directory outside the guard: {relative}:{}",
            line_of(text, temp.min(pid))
        ));
    }
    errors
}

/// The 1-based line the byte `at` sits on.
fn line_of(text: &str, at: usize) -> usize {
    match text.get(..at) {
        Some(prefix) => prefix.matches('\n').count().saturating_add(1),
        None => 1,
    }
}

/// Whether `relative` is test code: every file under a `tests` directory, an in-crate file that is
/// a simulation module, a tests module, or holds a `#[cfg(test)]` item behind it.
pub(super) fn is_test_code(relative: &str, tree: &Parsed) -> bool {
    if relative.split('/').any(|segment| segment == "tests") {
        return true;
    }
    let stem = Path::new(relative)
        .file_stem()
        .and_then(|stem| stem.to_str());
    if let Some(stem) = stem
        && (stem.starts_with("sim_") || stem.ends_with("_tests") || stem == "tests")
    {
        return true;
    }
    tree.root()
        .dfs()
        .any(|node| node.kind() == "attribute_item" && cfg_test(node.text().as_ref()))
}

/// Whether one attribute item is `#[cfg(test)]`.
fn cfg_test(text: &str) -> bool {
    let trimmed = text.trim();
    trimmed.starts_with("#[cfg(") && trimmed.contains("test")
}

/// The first temp source in `function`, by byte offset: a temp directory path, a system temp
/// directory string literal, `env!("TMPDIR")` or `var("TMPDIR")`.
fn first_temp(function: &AstNode<'_>, uses: &Uses, bound: &BTreeSet<String>) -> Option<usize> {
    for node in function.dfs() {
        let found = match node.kind().as_ref() {
            "scoped_identifier" | "identifier" => match path_of(&node) {
                Some(segments) => {
                    uses.is_temp(&segments, bound) || followed_by_tmpdir(&node, &segments)
                }
                None => false,
            },
            "string_literal" | "raw_string_literal" => match literal_text(node.text().as_ref()) {
                Some(content) => starts_temp(&content),
                None => false,
            },
            "macro_invocation" => is_tmpdir_macro(&node),
            "call_expression" => is_tmpdir_call(&node),
            _ => false,
        };
        if found {
            return Some(node.range().start);
        }
    }
    None
}

/// The first use of the process id in `function`, by byte offset, as a call or as a value.
fn first_pid(function: &AstNode<'_>, uses: &Uses, bound: &BTreeSet<String>) -> Option<usize> {
    for node in function.dfs() {
        if !matches!(node.kind().as_ref(), "scoped_identifier" | "identifier") {
            continue;
        }
        match path_of(&node) {
            Some(segments) if uses.is_pid(&segments, bound) => return Some(node.range().start),
            _ => {}
        }
    }
    None
}

/// Whether the path at `node` is an `env!` or `var` that names the temp directory, as a macro's
/// token tree spells it when the grammar does not knit the invocation into a node.
fn followed_by_tmpdir(node: &AstNode<'_>, segments: &[String]) -> bool {
    let Some(next) = node.next() else {
        return false;
    };
    let tree = if next.kind() == "!" {
        match next.next() {
            Some(tree) => tree,
            None => return false,
        }
    } else {
        next
    };
    if tree.kind() != "token_tree" {
        return false;
    }
    let is_source = match segments.last() {
        Some(last) => last == "var" || segments == ["env"],
        None => false,
    };
    if !is_source {
        return false;
    }
    match first_literal(&tree) {
        Some(content) => content == TMPDIR,
        None => false,
    }
}

/// Whether `node` invokes `env!("TMPDIR")`.
fn is_tmpdir_macro(node: &AstNode<'_>) -> bool {
    let mut children = node.named_children();
    let Some(name) = children.next() else {
        return false;
    };
    match collect_scoped(&name) {
        Some(segments) => {
            segments == ["env"]
                && match first_literal(node) {
                    Some(content) => content == TMPDIR,
                    None => false,
                }
        }
        None => false,
    }
}

/// Whether `node` calls `var("TMPDIR")` through any path, as `std::env::var` or an imported `var`.
fn is_tmpdir_call(node: &AstNode<'_>) -> bool {
    let Some(function) = node.field("function") else {
        return false;
    };
    let Some(segments) = collect_scoped(&function) else {
        return false;
    };
    if segments.last().map(String::as_str) != Some("var") {
        return false;
    }
    let Some(arguments) = node.field("arguments") else {
        return false;
    };
    match arguments.named_children().next() {
        Some(first) => match literal_text(first.text().as_ref()) {
            Some(content) => content == TMPDIR,
            None => false,
        },
        None => false,
    }
}

/// The content of the first string literal in `node`, if it holds one.
fn first_literal(node: &AstNode<'_>) -> Option<String> {
    node.dfs().find_map(|child| {
        if matches!(
            child.kind().as_ref(),
            "string_literal" | "raw_string_literal"
        ) {
            literal_text(child.text().as_ref())
        } else {
            None
        }
    })
}

/// A string or raw string literal's content, without its quotes or its `b`, `c` and `r#` markers.
fn literal_text(text: &str) -> Option<String> {
    let mut rest = text.trim();
    if let Some(after) = rest
        .strip_prefix('b')
        .or_else(|| rest.strip_prefix('c'))
        .filter(|after| after.starts_with('"') || after.starts_with('r'))
    {
        rest = after;
    }
    if let Some(after) = rest.strip_prefix('r') {
        let hashes = after
            .chars()
            .take_while(|character| *character == '#')
            .count();
        let body = after.get(hashes..)?;
        let inside = body.strip_prefix('"')?;
        let closing = hashes.saturating_add(1);
        let end = inside.len().checked_sub(closing)?;
        let tail = inside.get(end..)?;
        let expected = format!("\"{}", "#".repeat(hashes));
        if tail != expected {
            return None;
        }
        return inside.get(..end).map(String::from);
    }
    let inside = rest.strip_prefix('"')?;
    let content = inside.strip_suffix('"')?;
    Some(String::from(content))
}

/// The segments of the path `node` is, or that contains it. A name that defines an item, a field
/// or a method is no use of that name; a name in a `field_expression` is a field or method name.
fn path_of(node: &AstNode<'_>) -> Option<Vec<String>> {
    if names_definition(node) {
        return None;
    }
    match node.kind().as_ref() {
        "scoped_identifier" | "scoped_type_identifier" => collect_scoped(node),
        "identifier" => {
            let parent = node.parent()?;
            match parent.kind().as_ref() {
                "scoped_identifier" | "scoped_type_identifier" | "field_expression" => None,
                _ => Some(run_around(node)),
            }
        }
        _ => None,
    }
}

/// Whether `node` is the name a definition gives to the item it defines.
fn names_definition(node: &AstNode<'_>) -> bool {
    let Some(parent) = node.parent() else {
        return false;
    };
    if !DEFINITIONS.contains(&parent.kind().as_ref()) {
        return false;
    }
    match parent.field("name") {
        Some(name) => name.range() == node.range(),
        None => false,
    }
}

/// The path `node` names, segment by segment, when every part of it is a name or `::`.
pub(super) fn collect_scoped(node: &AstNode<'_>) -> Option<Vec<String>> {
    match node.kind().as_ref() {
        "metavariable" => Some(vec![String::from(node.text().as_ref())]),
        kind if SIMPLE_NAMES.contains(&kind) => Some(vec![String::from(node.text().as_ref())]),
        "scoped_identifier" | "scoped_type_identifier" => {
            let mut segments = Vec::new();
            for child in node.named_children() {
                segments.extend(collect_scoped(&child)?);
            }
            if segments.is_empty() {
                None
            } else {
                Some(segments)
            }
        }
        _ => None,
    }
}

/// The names of the longest `name::name` run that holds `node` among its siblings, as a macro's
/// token tree spells a path when the grammar does not knit it into one node.
fn run_around(node: &AstNode<'_>) -> Vec<String> {
    let mut before: Vec<String> = Vec::new();
    let mut cursor = node.prev();
    while let Some(separator) = cursor {
        if separator.kind() != "::" {
            break;
        }
        let Some(name) = separator.prev() else {
            break;
        };
        if name.kind() != "identifier" {
            break;
        }
        before.push(String::from(name.text().as_ref()));
        cursor = name.prev();
    }
    before.reverse();
    let mut segments = before;
    segments.push(String::from(node.text().as_ref()));
    let mut cursor = node.next();
    while let Some(separator) = cursor {
        if separator.kind() != "::" {
            break;
        }
        let Some(name) = separator.next() else {
            break;
        };
        if name.kind() != "identifier" {
            break;
        }
        segments.push(String::from(name.text().as_ref()));
        cursor = name.next();
    }
    segments
}

/// The names a function binds in a pattern: a `let`, a parameter, a `for`, a `match` arm or a
/// closure's arguments. A bare `id` or `temp_dir` a function binds is its own name, not the import.
fn bound_names(function: &AstNode<'_>) -> BTreeSet<String> {
    let mut names = BTreeSet::new();
    for node in function.dfs() {
        let pattern = match node.kind().as_ref() {
            "let_declaration" | "parameter" | "for_expression" | "match_arm" => {
                node.field("pattern")
            }
            "closure_expression" => node.field("parameters"),
            _ => None,
        };
        if let Some(pattern) = pattern {
            for child in pattern.dfs() {
                if child.kind() == "identifier" {
                    names.insert(String::from(child.text().as_ref()));
                }
            }
        }
    }
    names
}
