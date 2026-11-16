//! The probe's own resident-set sample (larry#269): `ps` reads one live process's CPU seconds and
//! resident set size, so an integration test measures its child without `/usr/bin/time`, without a
//! new crate and without the `unsafe` call `getrusage` would need. The PTY probe in
//! `tests/e2e_journey.rs` and the governance check in `tests/governance_journey.rs` both read it.

use std::process::Command;

/// CPU seconds and resident set size in KiB of `pid`, from `ps -p <pid> -o time=,rss=`. `None`
/// when the process has already gone or `ps` printed no reading, so a sampler can bound a short
/// run instead of failing on it.
pub fn sample(pid: &str) -> Option<(f64, u64)> {
    let output = Command::new("ps")
        .args(["-p", pid, "-o", "time=,rss="])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8(output.stdout).ok()?;
    let mut fields = text.split_whitespace();
    let time = fields.next()?;
    let rss: u64 = fields.next()?.parse().ok()?;
    let mut seconds = 0.0_f64;
    for part in time.split(':') {
        seconds = seconds.mul_add(60.0, part.parse::<f64>().ok()?);
    }
    Some((seconds, rss))
}

/// The sampler reads the process it runs in: a live pid has a resident set.
#[test]
fn a_sample_names_the_calling_process() -> Result<(), Box<dyn std::error::Error>> {
    let pid = std::process::id().to_string();
    let (_, rss) = sample(&pid).ok_or("ps reported no rss for this test process")?;
    assert!(rss > 0, "ps reported no rss: {rss}");
    Ok(())
}
