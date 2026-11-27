//! Coverage profile forwarding regression (larry#548): every `env_clear`
//! under `tests/` reaches `coverage::forward`, and a cleared child still sees
//! the profile path. Source: the LLVM profiling runtime (`LLVM_PROFILE_FILE`
//! names each process's profile output) and the `cargo llvm-cov` run that sets
//! it. Requirement: under coverage every spawned child counts toward the
//! report and no working tree gains a file; without it nothing changes.

#[path = "support/coverage.rs"]
mod coverage;
#[path = "support/temp_dir.rs"]
mod temp_dir;

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use temp_dir::TempDir;

/// Failures propagate instead of panicking.
type Check<T = ()> = Result<T, Box<dyn std::error::Error>>;

/// Collects every Rust source under `dir`, with `std::fs` only.
fn rust_sources(dir: &Path, out: &mut Vec<PathBuf>) -> Check {
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            rust_sources(&path, out)?;
        } else if path.extension().is_some_and(|ext| ext == "rs") {
            out.push(path);
        }
    }
    Ok(())
}

/// Whether `line` carries a forwarding call on the cleared builder.
fn is_forward(line: &str) -> bool {
    line.contains("forward(") || line.contains("forward_profile(")
}

/// Every `env_clear()` under `tests/` reaches a forwarding call in the same
/// statement chain. Names each `file:line` that does not.
#[test]
fn every_cleared_child_reaches_the_profile_forward() -> Check {
    let needle = format!("{}{}", "env_clear", "()");
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests");
    let mut sources = Vec::new();
    rust_sources(&root, &mut sources)?;
    sources.sort();
    assert!(
        !sources.is_empty(),
        "the scan found no test sources under {}",
        root.display()
    );
    let mut missing = Vec::new();
    for path in &sources {
        if path
            .file_name()
            .is_some_and(|name| name == "coverage_forward.rs")
        {
            continue;
        }
        let text = fs::read_to_string(path)?;
        let lines: Vec<&str> = text.lines().collect();
        for (index, line) in lines.iter().enumerate() {
            if !line.contains(&needle) {
                continue;
            }
            let reaches = lines
                .iter()
                .skip(index)
                .take(16)
                .any(|near| is_forward(near));
            if !reaches {
                missing.push(format!("{}:{}", path.display(), index + 1));
            }
        }
    }
    assert!(
        missing.is_empty(),
        "cleared builders that never reach coverage::forward: {}",
        missing.join(", ")
    );
    Ok(())
}

/// A cleared child sees the profile path its parent run sets. The temp path is
/// the value the child must carry when the parent run sets nothing else; when
/// the parent run already sets one (under `cargo llvm-cov`), that value wins.
#[test]
fn a_cleared_child_sees_its_profile_path() -> Check {
    let guard = TempDir::new()?;
    let temp = guard.path().join("child.profraw");
    let temp_text = temp.to_string_lossy().into_owned();
    let parent = std::env::var_os("LLVM_PROFILE_FILE");
    let wanted = match parent.as_deref() {
        Some(path) => path.to_string_lossy().into_owned(),
        None => temp_text.clone(),
    };
    let mut forwarded = Command::new("/usr/bin/env");
    forwarded
        .env_clear()
        .env("PATH", std::env::var("PATH")?)
        .env("LLVM_PROFILE_FILE", temp_text.as_str());
    coverage::forward(&mut forwarded);
    let seen = forwarded.output()?;
    assert!(
        seen.status.success(),
        "a forwarded child runs with a cleared environment"
    );
    let text = String::from_utf8_lossy(&seen.stdout);
    assert!(
        text.contains(wanted.as_str()),
        "the cleared child saw its profile path"
    );
    assert_eq!(
        coverage::expected(),
        parent,
        "the reader reports exactly what the run sets"
    );
    Ok(())
}
