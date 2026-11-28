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
pub(super) fn first_literal(node: &AstNode<'_>) -> Option<String> {
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

/// One source path a tree names, with the inline module names a `#[path]` attribute is rooted
/// through. An `include!` file is always read beside its file, so it carries no module names.
pub(super) struct Named {
    /// The path the source names.
    pub(super) name: String,
    /// The inline modules around the name, outermost first.
    pub(super) inline: Vec<String>,
}

/// Whether `relative` is a crate root, where an inline module's directory starts at the file's own
/// directory rather than one named for its stem (The Rust Reference, "Modules").
pub(super) fn crate_root(relative: &str) -> bool {
    let mut parts = relative.rsplit('/');
    let Some(name) = parts.next() else {
        return false;
    };
    let stem = match name.strip_suffix(".rs") {
        Some(stem) => stem,
        None => name,
    };
    matches!(stem, "lib" | "main" | "mod")
        || matches!(parts.next(), Some("tests" | "benches" | "examples" | "bin"))
}

/// Every source path `tree` names through a `#[path = "..."]` attribute or an `include!("...")`.
pub(super) fn follow_names(tree: &Parsed) -> Vec<Named> {
    let root = tree.root();
    let mut names = Vec::new();
    for node in root.dfs() {
        match node.kind().as_ref() {
            "attribute_item" => {
                let text = node.text();
                let trimmed = text.as_ref().trim_start();
                if let Some(rest) = trimmed.strip_prefix("#[path")
                    && rest.trim_start().starts_with('=')
                    && let Some(name) = first_literal(&node)
                {
                    names.push(Named {
                        name,
                        inline: inline_modules(&node),
                    });
                }
            }
            "macro_invocation" => {
                let mut children = node.named_children();
                let is_include = match children.next() {
                    Some(name) => collect_scoped(&name) == Some(vec![String::from("include")]),
                    None => false,
                };
                if is_include && let Some(name) = first_literal(&node) {
                    names.push(Named {
                        name,
                        inline: Vec::new(),
                    });
                }
            }
            _ => {}
        }
    }
    names
}

/// The inline module names `node` sits inside, outermost first: every `mod_item` above it whose
/// body is the container `node` reached it through. The module an attribute annotates is never
/// counted, so a `#[path]` on `mod x;` carries no name of its own.
fn inline_modules(node: &AstNode<'_>) -> Vec<String> {
    let mut names = Vec::new();
    let mut below = String::from(node.kind().as_ref());
    let mut cursor = node.parent();
    while let Some(ancestor) = cursor {
        if ancestor.kind() == "mod_item"
            && below == "declaration_list"
            && let Some(name) = ancestor.field("name")
        {
            names.push(String::from(name.text().as_ref()));
        }
        below = String::from(ancestor.kind().as_ref());
        cursor = ancestor.parent();
    }
    names.reverse();
    names
}

#[cfg(test)]
mod tests {
    use lgwks_ast::{Language, ParseError, try_parse};

    use super::{
        cfg_test, crate_root, follow_names, is_test_code, line_of, literal_text, starts_temp,
        verdicts,
    };

    /// The verdicts of one test source, read by the estate reader the rule uses.
    fn probe(source: &str) -> Result<Vec<String>, ParseError> {
        let tree = try_parse(source, Language::Rust)?;
        Ok(verdicts("tests/probe.rs", source, &tree))
    }

    /// The Rust Reference on paths and literals: the temp roots, the crate roots, the test
    /// gates, the line of a byte and each literal form are read exactly.
    #[test]
    fn sim_each_path_and_literal_form_reads_exactly() -> Result<(), ParseError> {
        for temp in ["/tmp/x", "/var/tmp/y", "/tmp"] {
            assert!(starts_temp(temp), "{temp}");
        }
        for other in ["/var/folders/x", "tmp/x", "/TMP/x", "//tmp", ""] {
            assert!(!starts_temp(other), "{other}");
        }
        for root in [
            "src/lib.rs",
            "src/main.rs",
            "src/a/mod.rs",
            "tests/x.rs",
            "tests/x",
            "benches/b.rs",
            "examples/e.rs",
            "src/bin/tool.rs",
        ] {
            assert!(crate_root(root), "{root}");
        }
        for nested in ["src/a.rs", "src/tests/deep/x.rs", "bin.rs", ""] {
            assert!(!crate_root(nested), "{nested}");
        }
        assert!(cfg_test("#[cfg(test)]"));
        assert!(cfg_test("  #[cfg(all(test, unix))] "));
        assert!(!cfg_test("#[cfg(unix)]"));
        assert!(!cfg_test("#[test]"));

        let plain = try_parse("fn f() {}\n", Language::Rust)?;
        for test_code in [
            "tests/a.rs",
            "src/sim_x.rs",
            "src/a_tests.rs",
            "src/tests.rs",
        ] {
            assert!(is_test_code(test_code, &plain), "{test_code}");
        }
        assert!(!is_test_code("src/a.rs", &plain));
        let gated = try_parse("#[cfg(test)]\nfn f() {}\n", Language::Rust)?;
        assert!(is_test_code("src/a.rs", &gated));

        assert_eq!(line_of("a\nb\nc", 0), 1);
        assert_eq!(line_of("a\nb\nc", 2), 2);
        assert_eq!(line_of("a\nb\nc", 5), 3);
        assert_eq!(line_of("a\nb\nc", 100), 1);

        for (spelled, content) in [
            ("\"x\"", Some("x")),
            ("b\"x\"", Some("x")),
            ("c\"x\"", Some("x")),
            ("br#\"x\"#", Some("x")),
            ("cr\"x\"", Some("x")),
            ("r##\"/tmp/\"x\"##", Some("/tmp/\"x")),
            ("r#\"x\"", None),
            ("r##\"\"#", None),
            ("r\"", None),
            ("r#", None),
            ("\"open", None),
            ("x", None),
            ("b", None),
        ] {
            assert_eq!(literal_text(spelled).as_deref(), content, "{spelled:?}");
        }
        Ok(())
    }

    /// The Rust Reference on macros: a `TMPDIR` read is a temp source however it is spelled, in
    /// a node or a token tree, and a read of anything else or a method named `var` is not.
    #[test]
    fn sim_only_a_read_of_the_temp_variable_is_a_temp_source() -> Result<(), ParseError> {
        let leak = ["a process id names a temp directory outside the guard: tests/probe.rs:2"];
        for source in [
            "fn f() -> String {\n    let set = format!(\"{}\", std::env::var(\"TMPDIR\"));\n    format!(\"{set}{}\", std::process::id())\n}\n",
            "fn f() -> String {\n    let dir = env!(\"TMPDIR\");\n    format!(\"{dir}{}\", std::process::id())\n}\n",
        ] {
            assert_eq!(probe(source)?, leak, "{source}");
        }
        for source in [
            "fn f() -> String {\n    let set = std::env::var();\n    format!(\"{set:?}{}\", std::process::id())\n}\n",
            "fn f() -> String {\n    let set = std::env::var(OTHER);\n    format!(\"{set:?}{}\", std::process::id())\n}\n",
            "fn f() -> String {\n    let set = std::env::var(123);\n    format!(\"{set:?}{}\", std::process::id())\n}\n",
            "fn f() -> String {\n    let set = std::env::var(\"HOME\");\n    format!(\"{set:?}{}\", std::process::id())\n}\n",
            "fn f() -> String {\n    let set = Helper.var(\"TMPDIR\");\n    format!(\"{set:?}{}\", std::process::id())\n}\n",
            "fn f() -> String {\n    let set = env!(\"HOME\");\n    format!(\"{set}{}\", std::process::id())\n}\n",
            "fn f() {\n    foo!(a :: 1);\n    foo!(:: b);\n    foo!(1 :: b);\n}\n",
        ] {
            assert_eq!(probe(source)?.len(), 0, "{source}");
        }
        Ok(())
    }

    /// The Rust Reference on modules and `include!`: each `#[path]` carries the inline modules
    /// around it, an `include!` carries none, and no other macro or attribute names a file.
    #[test]
    fn sim_only_a_path_attribute_or_an_include_names_a_module() -> Result<(), ParseError> {
        let named = |source: &str| -> Result<Vec<(String, Vec<String>)>, ParseError> {
            let tree = try_parse(source, Language::Rust)?;
            Ok(follow_names(&tree)
                .into_iter()
                .map(|found| (found.name, found.inline))
                .collect())
        };
        let found = named(
            "mod outer {\n    mod inner {\n        #[path = \"a.rs\"]\n        mod m;\n    }\n}\nfn f() { include!(\"b.rs\"); }\n",
        )?;
        let inline = vec![String::from("outer"), String::from("inner")];
        assert_eq!(
            found,
            [
                (String::from("a.rs"), inline),
                (String::from("b.rs"), Vec::new()),
            ]
        );
        for none in [
            "#[pathology = \"a.rs\"]\nmod m;\n",
            "#[path]\nmod m;\n",
            "fn f() { include_str!(\"a.txt\"); }\n",
            "fn f() { helper::pid(); }\n",
        ] {
            assert_eq!(named(none)?.len(), 0, "{none}");
        }
        Ok(())
    }
}
