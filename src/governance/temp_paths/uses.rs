//! The `use` declarations a file binds, and what each local name means to the temp-path rule
//! (larry#293, ADR 0058). A path the rule reads is resolved through an alias (`use std::process
//! as p`), a `self` import inside a group (`use std::{process::{self}}`), a glob
//! (`use std::process::*`) and a plain import of the function itself. Source: The Rust Reference
//! on use declarations.

use std::collections::{BTreeMap, BTreeSet};

use lgwks_ast::AstNode;

use super::syntax::collect_scoped;

/// What a `use` declaration makes a local name mean.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum Alias {
    /// The `std::process` module.
    Process,
    /// The `std::env` module.
    Env,
    /// The `std::process::id` call.
    Pid,
    /// The `std::env::temp_dir` call.
    Temp,
}

/// One path a `use` declaration reaches and the local name it binds.
pub(super) struct UseItem {
    /// The full path, segment by segment, without a leading `::`.
    pub(super) path: Vec<String>,
    /// The `as` name the declaration gives it, when one is given.
    pub(super) alias: Option<String>,
    /// Whether the declaration is a glob (`::*`), which binds every name of its module.
    pub(super) glob: bool,
}

/// The names a file's `use` declarations bind, by the meaning each has.
#[derive(Debug)]
pub(super) struct Uses {
    /// Each local name and its meaning.
    names: BTreeMap<String, Alias>,
    /// The modules a glob imports from.
    globs: BTreeSet<Alias>,
}

impl Uses {
    /// The `use` declarations the file holds outside its functions, resolved to local names.
    pub(super) fn of(root: &AstNode<'_>) -> Self {
        let mut uses = Self {
            names: BTreeMap::new(),
            globs: BTreeSet::new(),
        };
        for node in root.dfs() {
            if node.kind() != "use_declaration"
                || node
                    .ancestors()
                    .any(|parent| parent.kind() == "function_item")
            {
                continue;
            }
            let Some(argument) = node.field("argument") else {
                continue;
            };
            let mut base: Vec<String> = Vec::new();
            let mut items: Vec<UseItem> = Vec::new();
            clause(&argument, &mut base, &mut items);
            for item in items {
                if item.glob {
                    if let Some(alias) = item.imported_alias() {
                        uses.globs.insert(alias);
                    }
                    continue;
                }
                if let Some((local, alias)) = item.bound() {
                    uses.names.insert(local, alias);
                }
            }
        }
        uses
    }

    /// Whether `segments` names the process id: `std::process::id`, a module alias of
    /// `std::process`, or a bare `id` the file imports and this function does not rebind.
    pub(super) fn is_pid(&self, segments: &[String], bound: &BTreeSet<String>) -> bool {
        match *segments {
            [ref first, ref second, ref third]
                if first == "std" && second == "process" && third == "id" =>
            {
                true
            }
            [ref module, ref name] => {
                name == "id" && self.names.get(module) == Some(&Alias::Process)
            }
            [ref name] => {
                (self.names.get(name) == Some(&Alias::Pid)
                    || name == "id" && self.globs.contains(&Alias::Process))
                    && !bound.contains(name)
            }
            _ => false,
        }
    }

    /// Whether `segments` names the temp directory: `std::env::temp_dir`, a module alias of
    /// `std::env`, or a bare `temp_dir` the file imports and this function does not rebind.
    pub(super) fn is_temp(&self, segments: &[String], bound: &BTreeSet<String>) -> bool {
        match *segments {
            [ref first, ref second, ref third]
                if first == "std" && second == "env" && third == "temp_dir" =>
            {
                true
            }
            [ref module, ref name] => {
                name == "temp_dir" && self.names.get(module) == Some(&Alias::Env)
            }
            [ref name] => {
                (self.names.get(name) == Some(&Alias::Temp)
                    || name == "temp_dir" && self.globs.contains(&Alias::Env))
                    && !bound.contains(name)
            }
            _ => false,
        }
    }
}

impl UseItem {
    /// The meaning a glob item imports from.
    fn imported_alias(&self) -> Option<Alias> {
        match *self.path.as_slice() {
            [ref first, ref second] if first == "std" && second == "process" => {
                Some(Alias::Process)
            }
            [ref first, ref second] if first == "std" && second == "env" => Some(Alias::Env),
            _ => None,
        }
    }

    /// The local name the item binds and its meaning, when the path is the process id, the temp
    /// directory or the module that holds one of them.
    fn bound(&self) -> Option<(String, Alias)> {
        let local = match self.alias {
            Some(ref name) => name.clone(),
            None => self.path.last()?.clone(),
        };
        let alias = match *self.path.as_slice() {
            [ref first, ref second, ref third]
                if first == "std" && second == "process" && third == "id" =>
            {
                Alias::Pid
            }
            [ref first, ref second, ref third]
                if first == "std" && second == "env" && third == "temp_dir" =>
            {
                Alias::Temp
            }
            [ref first, ref second] if first == "std" && second == "process" => Alias::Process,
            [ref first, ref second] if first == "std" && second == "env" => Alias::Env,
            _ => return None,
        };
        Some((local, alias))
    }
}

/// Parses one `use` tree at `node`, extending `base` with the segments it names.
pub(super) fn clause(node: &AstNode<'_>, base: &mut Vec<String>, items: &mut Vec<UseItem>) {
    match node.kind().as_ref() {
        "identifier" | "crate" | "super" => {
            let mut segments = base.clone();
            segments.push(String::from(node.text().as_ref()));
            items.push(UseItem {
                path: segments,
                alias: None,
                glob: false,
            });
        }
        "self" => items.push(UseItem {
            path: base.clone(),
            alias: None,
            glob: false,
        }),
        "scoped_identifier" => {
            let Some(segments) = collect_scoped(node) else {
                return;
            };
            let mut path = base.clone();
            path.extend(segments);
            items.push(UseItem {
                path,
                alias: None,
                glob: false,
            });
        }
        "use_as_clause" => {
            let (Some(path), Some(alias)) = (node.field("path"), node.field("alias")) else {
                return;
            };
            let mut segments = base.clone();
            if path.kind() != "self" {
                let Some(rest) = collect_scoped(&path) else {
                    return;
                };
                segments.extend(rest);
            }
            items.push(UseItem {
                path: segments,
                alias: Some(String::from(alias.text().as_ref())),
                glob: false,
            });
        }
        "scoped_use_list" => {
            let (Some(path), Some(list)) = (node.field("path"), node.field("list")) else {
                return;
            };
            let Some(rest) = collect_scoped(&path) else {
                return;
            };
            let mut segments = base.clone();
            segments.extend(rest);
            clause(&list, &mut segments, items);
        }
        "use_list" => {
            for child in node.named_children() {
                clause(&child, base, items);
            }
        }
        "use_wildcard" => {
            let Some(path) = node.named_children().next() else {
                return;
            };
            let Some(rest) = collect_scoped(&path) else {
                return;
            };
            let mut segments = base.clone();
            segments.extend(rest);
            items.push(UseItem {
                path: segments,
                alias: None,
                glob: true,
            });
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use lgwks_ast::{Language, ParseError, try_parse};

    use super::Uses;

    /// The names `source` binds, read by the estate reader the rule uses.
    fn uses_of(source: &str) -> Result<Uses, ParseError> {
        Ok(Uses::of(&try_parse(source, Language::Rust)?.root()))
    }

    /// Whether `path`, written `a::b`, names the process id and the temp directory, in a function
    /// that rebinds `bound`.
    fn meaning(uses: &Uses, path: &str, bound: &[&str]) -> (bool, bool) {
        let segments: Vec<String> = path.split("::").map(String::from).collect();
        let bound: BTreeSet<String> = bound.iter().map(|name| String::from(*name)).collect();
        (
            uses.is_pid(&segments, &bound),
            uses.is_temp(&segments, &bound),
        )
    }

    /// The Rust Reference on use declarations: an alias, a `self` group, a glob and a plain
    /// import each give the local name its meaning, and a rebound bare name shadows only itself.
    #[test]
    fn sim_every_use_spelling_gives_the_local_name_its_meaning() -> Result<(), ParseError> {
        const PID: (bool, bool) = (true, false);
        const TEMP: (bool, bool) = (false, true);
        const NEITHER: (bool, bool) = (false, false);
        let none = uses_of("fn f() {}\n")?;
        assert_eq!(meaning(&none, "std::process::id", &[]), PID);
        assert_eq!(meaning(&none, "std::env::temp_dir", &[]), TEMP);
        for other in ["std::process", "other::process::id", "id", "temp_dir"] {
            assert_eq!(meaning(&none, other, &[]), NEITHER, "{other}");
        }

        let aliased = uses_of("use std::process as p;\nuse std::env as e;\n")?;
        assert_eq!(meaning(&aliased, "p::id", &["id"]), PID);
        assert_eq!(meaning(&aliased, "e::temp_dir", &["temp_dir"]), TEMP);
        assert_eq!(meaning(&aliased, "e::id", &[]), NEITHER);
        assert_eq!(meaning(&aliased, "p::temp_dir", &[]), NEITHER);

        let plain = uses_of("use std::process::id;\nuse std::env::temp_dir as td;\n")?;
        assert_eq!(meaning(&plain, "id", &[]), PID);
        assert_eq!(meaning(&plain, "td", &["id"]), TEMP);
        assert_eq!(meaning(&plain, "id", &["id"]), NEITHER);
        assert_eq!(meaning(&plain, "td", &["td"]), NEITHER);
        assert_eq!(meaning(&plain, "temp_dir", &[]), NEITHER);

        let grouped = uses_of("use std::{process::{self}, env::{self as e}};\n")?;
        assert_eq!(meaning(&grouped, "process::id", &[]), PID);
        assert_eq!(meaning(&grouped, "e::temp_dir", &[]), TEMP);

        let glob = uses_of("use std::process::*;\nuse std::env::*;\n")?;
        assert_eq!(meaning(&glob, "id", &[]), PID);
        assert_eq!(meaning(&glob, "temp_dir", &[]), TEMP);
        assert_eq!(meaning(&glob, "id", &["id"]), NEITHER);
        assert_eq!(meaning(&glob, "temp_dir", &["temp_dir"]), NEITHER);
        Ok(())
    }

    /// CWE-693: a path that only resembles the standard one, and a `use` inside a function, bind
    /// no name at all.
    #[test]
    fn sim_a_lookalike_use_binds_nothing() -> Result<(), ParseError> {
        for source in [
            "fn f() { use std::process::id; }\n",
            "use crate::process::id;\n",
            "use other::std::process::id;\n",
            "use super::env::temp_dir;\n",
            "use std::process::exit;\n",
            "use std::env::var_os;\n",
            "use std::process::{Command, Stdio};\n",
            "use std::collections::*;\n",
            "use std::process::id::*;\n",
            "use *;\n",
        ] {
            let uses = uses_of(source)?;
            assert!(
                uses.names.is_empty() && uses.globs.is_empty(),
                "{source}: {uses:?}"
            );
        }
        Ok(())
    }
}
