//! The probe's toolchain section (larry#274): one fixed argv per tool, read from
//! its own stream, `ok` only on exit 0. Split from `executor::probe`, which
//! owns the probe value and every other section, so each file stays under the
//! size law.

use std::path::Path;

use lgwks_std::json::{Value, json};

use super::{MAX_ERROR_CHARS, ProbeError, Runner, sanitize, version_line};

/// One tool's fixed argv and which stream carries its version: `go version`
/// prints to stdout, `java -version` to stderr, the rest `--version` to
/// stdout (larry#274, sixth pass: one argv per tool, from a table).
struct ToolSpec {
    /// The tool's name on PATH.
    tool: &'static str,
    /// The fixed arguments after the tool name.
    args: &'static [&'static str],
    /// Whether the version is read from stderr instead of stdout.
    from_stderr: bool,
}

/// The fixed version argv per tool, in probe order.
const TOOL_SPECS: [ToolSpec; 8] = [
    ToolSpec {
        tool: "rustc",
        args: &["--version"],
        from_stderr: false,
    },
    ToolSpec {
        tool: "cargo",
        args: &["--version"],
        from_stderr: false,
    },
    ToolSpec {
        tool: "node",
        args: &["--version"],
        from_stderr: false,
    },
    ToolSpec {
        tool: "npm",
        args: &["--version"],
        from_stderr: false,
    },
    ToolSpec {
        tool: "python3",
        args: &["--version"],
        from_stderr: false,
    },
    ToolSpec {
        tool: "cc",
        args: &["--version"],
        from_stderr: false,
    },
    ToolSpec {
        tool: "go",
        args: &["version"],
        from_stderr: false,
    },
    ToolSpec {
        tool: "java",
        args: &["-version"],
        from_stderr: true,
    },
];

/// What one probe spawn did: its exit status (or `None` for a signal), whether
/// the bound killed it, and both streams. A tool line is `ok` only when the
/// call exits 0; otherwise it is the typed failure with the exit status (or
/// `timed_out`), never the stderr text as a version (larry#274, sixth pass).
#[derive(Debug, Clone)]
pub(crate) struct ProbeCall {
    /// The process exit code, or `None` when a signal ended it.
    pub(crate) exit_code: Option<i32>,
    /// Whether the probe bound killed the call.
    pub(crate) timed_out: bool,
    /// What the call wrote to standard output.
    pub(crate) stdout: String,
    /// What the call wrote to standard error.
    pub(crate) stderr: String,
}

impl ProbeCall {
    /// A call that exited 0 with `text` on standard output: the simulations'
    /// scripted answer, never a host spawn.
    #[cfg(test)]
    #[must_use]
    pub(crate) fn ok(text: &str) -> ProbeCall {
        ProbeCall {
            exit_code: Some(0),
            timed_out: false,
            stdout: text.to_owned(),
            stderr: String::new(),
        }
    }

    /// A call that ended with `status` and both streams kept.
    #[must_use]
    pub(crate) fn failed(status: Option<i32>, stdout: &str, stderr: &str) -> ProbeCall {
        ProbeCall {
            exit_code: status,
            timed_out: false,
            stdout: stdout.to_owned(),
            stderr: stderr.to_owned(),
        }
    }

    /// A call the probe bound killed before it ended: the simulations'
    /// scripted answer for the never-exits fixture.
    #[cfg(test)]
    #[must_use]
    pub(crate) fn timed_out() -> ProbeCall {
        ProbeCall {
            exit_code: None,
            timed_out: true,
            stdout: String::new(),
            stderr: String::new(),
        }
    }

    /// Marks whether the probe bound killed the call.
    #[must_use]
    pub(crate) fn timed(mut self, timed_out: bool) -> ProbeCall {
        self.timed_out = timed_out;
        self
    }

    /// The call's output for the sections that read one stream: standard
    /// output when it has any, else standard error (the old `df`/`git`
    /// behaviour; the toolchain section reads its own stream per tool).
    #[must_use]
    pub(crate) fn text(&self) -> &str {
        if self.stdout.is_empty() {
            self.stderr.as_str()
        } else {
            self.stdout.as_str()
        }
    }
}

/// The toolchain section: one entry per fixed tool, in [`TOOL_SPECS`] order, with
/// its version line or the typed reason it has none. A line is `ok` only when
/// its call exits 0; a non-zero exit, a signal or the bound kill is the typed
/// failure naming the status, never the stderr text as a version.
pub(crate) fn toolchains(run: Runner<'_>, root: &Path) -> Value {
    let mut tools: Vec<Value> = Vec::new();
    for spec in TOOL_SPECS {
        let mut argv = Vec::with_capacity(spec.args.len().saturating_add(1));
        argv.push(spec.tool.to_owned());
        argv.extend(spec.args.iter().map(|arg| (*arg).to_owned()));
        let entry = match run(&argv, root) {
            Ok(call) => finish_tool(&spec, &call),
            Err(cause) => refused(spec.tool, &cause),
        };
        tools.push(entry);
    }
    Value::Array(tools)
}

/// One tool's entry from what its call did: `ok` with its version line only
/// on exit 0, else the typed failure with the exit status or the timeout.
fn finish_tool(spec: &ToolSpec, call: &ProbeCall) -> Value {
    if call.timed_out {
        return refused(spec.tool, &ProbeError::Command(String::from("timed out")));
    }
    if call.exit_code != Some(0) {
        let reason = match call.exit_code {
            Some(status) => format!("exit status {status}"),
            None => String::from("ended by a signal"),
        };
        return refused(spec.tool, &ProbeError::Command(reason));
    }
    let text = if spec.from_stderr {
        call.stderr.as_str()
    } else {
        call.stdout.as_str()
    };
    match version_line(text) {
        Ok(version) => json!({
            "tool": spec.tool,
            "ok": true,
            "version": version,
        }),
        Err(cause) => refused(spec.tool, &cause),
    }
}

/// One refused tool: its name and the typed reason, never a secret.
fn refused(tool: &str, cause: &ProbeError) -> Value {
    json!({
        "tool": tool,
        "ok": false,
        "error": sanitize(&cause.to_string(), MAX_ERROR_CHARS),
    })
}
