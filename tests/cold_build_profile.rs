//! Regression coverage for Larry's cold-build profile contract.

use std::error::Error;
use std::fs;
use std::path::Path;

type TestResult = Result<(), Box<dyn Error>>;

/// The default dev codegen level avoids optimizing every dependency during a cold binary build.
#[test]
fn cold_build_profile_uses_unoptimized_dev_profile() -> TestResult {
    let manifest = fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml"))?;
    let dev = manifest
        .split_once("[profile.dev]")
        .ok_or("the dev profile is declared")?
        .1
        .split_once("[profile.release]")
        .ok_or("the release profile follows the dev profile")?
        .0;
    assert!(
        !dev.lines()
            .any(|line| line.trim_start().starts_with("opt-level")),
        "the dev profile must use Cargo's default opt-level: {dev}"
    );
    assert!(
        !manifest.contains("[profile.dev.package.\"*\"]"),
        "the dev profile must not optimize every dependency"
    );
    Ok(())
}
