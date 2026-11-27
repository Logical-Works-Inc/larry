//! Regression coverage for Larry's cold-build profile contract.

use std::error::Error;
use std::fs;
use std::path::Path;

type TestResult = Result<(), Box<dyn Error>>;

fn section<'a>(manifest: &'a str, name: &str) -> Result<&'a str, Box<dyn Error>> {
    Ok(manifest
        .split_once(name)
        .ok_or_else(|| format!("the section is declared: {name}"))?
        .1
        .split_once("\n[")
        .ok_or_else(|| format!("the next section follows {name}"))?
        .0)
}

/// The chosen profile keeps workspace and dependency code at opt-level one and build scripts at zero.
#[test]
fn dev_profile_uses_candidate_b_profile() -> TestResult {
    let manifest = fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml"))?;
    let dev = section(&manifest, "[profile.dev]")?;
    assert!(
        dev.lines().any(|line| line.trim() == "opt-level = 1"),
        "the workspace profile must use opt-level 1: {dev}"
    );
    let dependencies = section(&manifest, "[profile.dev.package.\"*\"]")?;
    assert!(
        dependencies
            .lines()
            .any(|line| line.trim() == "opt-level = 1"),
        "the dependency profile must use opt-level 1: {dependencies}"
    );
    for line in manifest.lines() {
        if let Some(package) = line.strip_prefix("[profile.dev.package.\"") {
            let package = package
                .strip_suffix("\"]")
                .ok_or_else(|| format!("the package profile header closes: {package}"))?;
            assert_eq!(
                package, "*",
                "the suite-safe profile must not add a selective package override: {package}"
            );
        }
    }
    let build_override = section(&manifest, "[profile.dev.build-override]")?;
    assert!(
        build_override
            .lines()
            .any(|line| line.trim() == "opt-level = 0"),
        "the build-script profile must use opt-level 0: {build_override}"
    );
    Ok(())
}
