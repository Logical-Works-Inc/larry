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
