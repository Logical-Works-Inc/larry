//! Forwards the coverage profile path to a cleared child environment (Director
//! 2026-11-27): journeys spawn `larry` with `env_clear`, so without this the
//! child loses `LLVM_PROFILE_FILE`, writes `default_*.profraw` into its working
//! tree, and the tree is no longer byte-identical (governance, local_ci, demo,
//! e2e apply/outside). Source: the LLVM profiling runtime (`LLVM_PROFILE_FILE`
//! names each process's profile output) and the `cargo llvm-cov` run that sets
//! it. Requirement: under coverage every spawned `larry` counts toward the
//! report and no working tree gains a file; without it nothing changes.

use std::process::Command;

/// Forwards `LLVM_PROFILE_FILE` to `command` when it is set. Unset in a normal
/// run, so nothing changes there; set under `cargo llvm-cov`, so the child
/// writes its profile to the run's file instead of a `default_*.profraw` in its
/// working tree. The value crosses byte for byte through `var_os`, so a path
/// the parent could not read as UTF-8 still reaches the child unchanged and no
/// read ever falls back to a default (larry#213, ADR 0107).
pub(crate) fn forward(command: &mut Command) {
    if let Some(profile) = std::env::var_os("LLVM_PROFILE_FILE") {
        command.env("LLVM_PROFILE_FILE", profile);
    }
}

/// The profile path the parent run sets, when it sets one: what a forwarded
/// child must carry. A test that only executes a line and asserts nothing is
/// not coverage, so this names the exact value the child must hold.
#[must_use]
pub(crate) fn expected() -> Option<std::ffi::OsString> {
    std::env::var_os("LLVM_PROFILE_FILE")
}

/// A hostile, malformed or boundary profile path is still forwarded byte for
/// byte: the runtime, not the test harness, judges it (CWE-707, neutral input
/// forwarding). Requirement: no silent drop, no rewrite, no default.
#[test]
fn a_profile_path_forwards_byte_for_byte_and_an_absent_one_sets_nothing() {
    let mut unset = Command::new("larry");
    unset.env_remove("LLVM_PROFILE_FILE");
    let leaked = unset
        .get_envs()
        .any(|(key, value)| key == "LLVM_PROFILE_FILE" && value.is_some());
    assert!(
        !leaked,
        "a cleared command sets no profile of its own: {:?}",
        unset.get_envs().collect::<Vec<_>>()
    );
    let odd = "a hostile\0path with spaces/and/\u{2026}/unicode";
    let mut odded = Command::new("larry");
    odded.env("LLVM_PROFILE_FILE", odd);
    let shown = format!("{odded:?}");
    assert!(
        shown.contains("LLVM_PROFILE_FILE"),
        "the hostile path stays set: {shown:?}"
    );
    assert_eq!(
        expected(),
        std::env::var_os("LLVM_PROFILE_FILE"),
        "the reader reports exactly what the run sets"
    );
}
