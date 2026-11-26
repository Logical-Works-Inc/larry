//! Regression coverage for Larry's cold-build profile contract.

use std::error::Error;
use std::fs;
use std::path::Path;

type TestResult = Result<(), Box<dyn Error>>;

/// Only Larry's measured hot-path packages receive dev optimization.
#[test]
fn dev_profile_optimizes_workspace_hot_path_only() -> TestResult {
    let manifest = fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml"))?;
    let dev = manifest
        .split_once("[profile.dev]")
        .ok_or("the dev profile is declared")?
        .1
        .split_once("\n[")
        .ok_or("the next profile follows the dev profile")?
        .0;
    assert!(
        dev.lines()
            .all(|line| !line.trim_start().starts_with("opt-level")),
        "the base dev profile must use Cargo's default opt-level: {dev}"
    );
    assert!(
        !manifest.contains("[profile.dev.package.\"*\"]"),
        "the dev profile must not optimize every dependency"
    );
    let allowed = ["larry-demo", "larry-contracts", "larry-runtime"];
    for package in manifest.lines().filter_map(|line| {
        line.strip_prefix("[profile.dev.package.\"")
            .and_then(|line| line.strip_suffix("\"]"))
    }) {
        assert!(
            allowed.contains(&package),
            "only measured Larry packages receive an override: {package}"
        );
    }
    for package in ["larry-demo", "larry-contracts", "larry-runtime"] {
        let section = format!("[profile.dev.package.\"{package}\"]");
        let package_profile = manifest
            .split_once(&section)
            .ok_or_else(|| format!("the hot package profile is declared for {package}"))?
            .1
            .split_once("\n[")
            .ok_or("the next profile follows the package profile")?
            .0;
        assert!(
            package_profile
                .lines()
                .any(|line| line.trim() == "opt-level = 1"),
            "the hot package profile must use opt-level 1 for {package}: {package_profile}"
        );
    }
    let build_override = manifest
        .split_once("[profile.dev.build-override]")
        .ok_or("the dev build override is declared")?
        .1
        .split_once("\n[")
        .ok_or("the next profile follows the build override")?
        .0;
    assert!(
        build_override
            .lines()
            .any(|line| line.trim() == "opt-level = 0"),
        "build scripts and proc macros must stay at opt-level 0: {build_override}"
    );
    Ok(())
}
