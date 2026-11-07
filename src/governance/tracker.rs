//! The progress tracker: `tools/tracker/tracker.json` rendered into `template.html` as
//! `index.html`, byte-identical to the Python generator it replaced, and the drift checks the
//! governance gate runs on it. Writes are atomic, so a killed write leaves the previous whole page.

use std::collections::BTreeSet;
use std::fmt;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use lgwks_std::json::{self, Value};

use super::patterns::Patterns;
use super::{pyjson, pystr};

/// The directory holding the tracker files, relative to the repository root.
const TRACKER_DIRECTORY: [&str; 2] = ["tools", "tracker"];
/// The hand-maintained data file.
const DATA_NAME: &str = "tracker.json";
/// The page template.
const TEMPLATE_NAME: &str = "template.html";
/// The rendered page.
const OUTPUT_NAME: &str = "index.html";
/// The token replaced by the data.
const DATA_TOKEN: &str = "__TRACKER_DATA__";
/// Largest admitted data file, template and page, in bytes.
const MAX_TRACKER_BYTES: u64 = 4 * 1024 * 1024;
/// Distinguishes the temporary files of concurrent writes inside one process.
static WRITE_SEQUENCE: AtomicU64 = AtomicU64::new(0);

/// Every way the tracker render or write can fail. Each variant names what failed.
#[derive(Debug)]
#[non_exhaustive]
pub enum TrackerError {
    /// A file could not be read.
    Read {
        /// The file that failed.
        path: PathBuf,
        /// The io cause, as text.
        cause: String,
    },
    /// A file exceeds the 4 MiB bound.
    TooLarge {
        /// The file that is too large.
        path: PathBuf,
        /// The admitted bound in bytes.
        limit: u64,
    },
    /// A file is not valid UTF-8; the text is CPython's codec message.
    NotUtf8 {
        /// The file that is not UTF-8.
        path: PathBuf,
        /// The decode error, as CPython words it.
        cause: String,
    },
    /// The data file is not valid JSON.
    Data(json::Error),
    /// The template lacks the data token.
    MissingToken,
    /// The rendered page could not be written.
    Write {
        /// The file that failed.
        path: PathBuf,
        /// The io cause, as text.
        cause: String,
    },
}

impl fmt::Display for TrackerError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::Read {
                ref path,
                ref cause,
            } => write!(f, "cannot read {}: {cause}", path.display()),
            Self::TooLarge { ref path, limit } => {
                write!(f, "{} exceeds {limit} bytes", path.display())
            }
            Self::NotUtf8 { ref cause, .. } => f.write_str(cause),
            Self::Data(ref error) => write!(f, "{error}"),
            Self::MissingToken => f.write_str("template is missing the __TRACKER_DATA__ token"),
            Self::Write {
                ref path,
                ref cause,
            } => write!(f, "cannot write {}: {cause}", path.display()),
        }
    }
}

impl std::error::Error for TrackerError {}

/// The tracker directory under `root`.
fn directory(root: &Path) -> PathBuf {
    TRACKER_DIRECTORY
        .iter()
        .fold(root.to_path_buf(), |path, part| path.join(part))
}

/// Reads one bounded UTF-8 file of the tracker directory.
fn read_bounded(path: &Path) -> Result<String, TrackerError> {
    let read_error = |cause: std::io::Error| TrackerError::Read {
        path: path.to_path_buf(),
        cause: cause.to_string(),
    };
    let metadata = fs::metadata(path).map_err(read_error)?;
    if metadata.len() > MAX_TRACKER_BYTES {
        return Err(TrackerError::TooLarge {
            path: path.to_path_buf(),
            limit: MAX_TRACKER_BYTES,
        });
    }
    let bytes = fs::read(path).map_err(read_error)?;
    String::from_utf8(bytes).map_err(|error| TrackerError::NotUtf8 {
        path: path.to_path_buf(),
        cause: pystr::utf8_error(error.as_bytes(), &error.utf8_error()),
    })
}

/// The page for `data` rendered into `template`: the key-sorted, ASCII-escaped dump with every
/// `<` escaped, so no value can close the surrounding `script` element.
fn render_with(data: &Value, template: &str) -> Result<String, TrackerError> {
    if !template.contains(DATA_TOKEN) {
        return Err(TrackerError::MissingToken);
    }
    let payload = pyjson::dumps(data, Some(2)).replace('<', "\\u003c");
    Ok(template.replace(DATA_TOKEN, &payload))
}

/// Parses the tracker data file under `root`.
fn load_data(root: &Path) -> Result<Value, TrackerError> {
    let text = read_bounded(&directory(root).join(DATA_NAME))?;
    json::from_str::<Value>(&text).map_err(TrackerError::Data)
}

/// Renders the page for the tracker under `root`: the template first, then the data.
pub(crate) fn render(root: &Path) -> Result<String, TrackerError> {
    let template = read_bounded(&directory(root).join(TEMPLATE_NAME))?;
    let data = load_data(root)?;
    render_with(&data, &template)
}

/// Writes the rendered page atomically, a fresh temporary sibling then a rename over
/// `index.html`, and returns its byte length. A failed write removes its temporary file.
///
/// # Errors
/// Returns a [`TrackerError`] when the render or the write fails.
pub fn write(root: &Path) -> Result<usize, TrackerError> {
    let page = render(root)?;
    let directory = directory(root);
    let output = directory.join(OUTPUT_NAME);
    let sequence = WRITE_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let temporary = directory.join(format!(
        ".{OUTPUT_NAME}.{}.{sequence}.tmp",
        std::process::id()
    ));
    let written =
        write_new(&temporary, page.as_bytes()).and_then(|()| fs::rename(&temporary, &output));
    match written {
        Ok(()) => Ok(page.len()),
        Err(cause) => {
            let removed = fs::remove_file(&temporary);
            let path = if removed.is_ok() { output } else { temporary };
            Err(TrackerError::Write {
                path,
                cause: cause.to_string(),
            })
        }
    }
}

/// Creates `path`, refusing one that already exists, and writes and syncs `bytes` into it.
fn write_new(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)?;
    file.write_all(bytes)?;
    file.sync_all()
}

/// Whether the committed `index.html` is a file equal to the render; a missing page is stale.
///
/// # Errors
/// Returns a [`TrackerError`] when the render fails or the committed page cannot be read.
pub fn is_current(root: &Path) -> Result<bool, TrackerError> {
    let expected = render(root)?;
    let output = directory(root).join(OUTPUT_NAME);
    if !output.is_file() {
        return Ok(false);
    }
    Ok(read_bounded(&output)? == expected)
}

/// Port of the generator's `tracker_errors`: an unreadable or unrenderable tracker, a stale or
/// missing page, a cell inventory that differs from `spec`, and falsifier ids or totals that drifted.
pub(crate) fn drift_errors(root: &Path, spec: &str, patterns: &Patterns) -> Vec<String> {
    let data = match load_data(root) {
        Ok(data) => data,
        Err(error) => return vec![format!("progress tracker: unreadable data ({error})")],
    };
    let expected = match render(root) {
        Ok(page) => page,
        Err(error) => return vec![format!("progress tracker: cannot render ({error})")],
    };
    let mut errors = Vec::new();
    let rendered = directory(root).join(OUTPUT_NAME);
    if !rendered.is_file() {
        errors.push(String::from(
            "progress tracker: tools/tracker/index.html is missing",
        ));
    } else {
        match read_bounded(&rendered) {
            Ok(actual) if actual == expected => {}
            Ok(_) => errors.push(String::from(
                "progress tracker: tools/tracker/index.html is stale — regenerate with `larry tracker`",
            )),
            Err(error) => return vec![format!("progress tracker: {error}")],
        }
    }
    let declared_cells: Vec<String> = spec_rows(spec, patterns, &patterns.tracker_cell)
        .iter()
        .map(|id| patterns.classes.repr(id))
        .collect();
    let tracked_cells = ids_of(&data, "cells", patterns);
    if !declared_cells.is_empty() && tracked_cells != declared_cells {
        let declared: BTreeSet<&String> = declared_cells.iter().collect();
        let tracked: BTreeSet<&String> = tracked_cells.iter().collect();
        let missing: Vec<String> = declared
            .difference(&tracked)
            .map(|id| String::from(id.as_str()))
            .collect();
        let extra: Vec<String> = tracked
            .difference(&declared)
            .map(|id| String::from(id.as_str()))
            .collect();
        errors.push(format!(
            "progress tracker: cell inventory does not match specification.md (missing {}, unexpected {}, order must match the specification)",
            or_none(&missing),
            or_none(&extra),
        ));
    }
    let declared_falsifiers: BTreeSet<String> =
        spec_rows(spec, patterns, &patterns.tracker_falsifier)
            .iter()
            .map(|id| patterns.classes.repr(id))
            .collect();
    if !declared_falsifiers.is_empty() {
        let tracked: BTreeSet<String> = ids_of(&data, "falsifiers", patterns).into_iter().collect();
        let unknown: Vec<String> = tracked.difference(&declared_falsifiers).cloned().collect();
        if !unknown.is_empty() {
            errors.push(format!(
                "progress tracker: falsifier ids not in specification.md: [{}]",
                unknown.join(", ")
            ));
        }
        let total = data.get("falsifier_total");
        if !counts_equal(total, declared_falsifiers.len()) {
            let shown = match total {
                Some(value) => pyjson::python_str(value, &patterns.classes),
                None => String::from("None"),
            };
            errors.push(format!(
                "progress tracker: falsifier_total is {shown}, specification.md declares {}",
                declared_falsifiers.len()
            ));
        }
    }
    errors
}

/// The first-seen unique group 1 of `row` `re.match`ed against each line of `spec`.
fn spec_rows(spec: &str, patterns: &Patterns, row: &lgwks_std::pattern::Regex) -> Vec<String> {
    let mut found: Vec<String> = Vec::new();
    for line in pystr::splitlines(spec) {
        match patterns.group(row, line, 1) {
            Some(id) if !found.iter().any(|seen| seen == id) => found.push(String::from(id)),
            _ => {}
        }
    }
    found
}

/// `[item.get("id") for item in data.get(key, [])]`, each id as its Python `repr` so a string
/// id compares by value and any other id (`None`, a number) can never equal a declared one.
fn ids_of(data: &Value, key: &str, patterns: &Patterns) -> Vec<String> {
    let Some(items) = data.get(key).and_then(Value::as_array) else {
        return Vec::new();
    };
    items
        .iter()
        .map(|item| match item.get("id") {
            Some(id) => pyjson::python_repr(id, &patterns.classes),
            None => String::from("None"),
        })
        .collect()
}

/// `repr(items) or 'none'` for ids already in their `repr` form.
fn or_none(items: &[String]) -> String {
    if items.is_empty() {
        String::from("none")
    } else {
        format!("[{}]", items.join(", "))
    }
}

/// `total == count` as Python compares a `json.load` value with an `int`: an integer or an
/// integral float equal to `count`. A boolean is not a count here (ADR 0010).
fn counts_equal(total: Option<&Value>, count: usize) -> bool {
    let Some(value) = total else {
        return false;
    };
    let Value::Number(ref number) = *value else {
        return false;
    };
    if number.as_u64().is_some() || number.as_i64().is_some() {
        return match u64::try_from(count) {
            Ok(wide) => number.as_u64() == Some(wide),
            Err(_) => false,
        };
    }
    number
        .as_f64()
        .is_some_and(|float| pyjson::float_repr(float) == format!("{count}.0"))
}
