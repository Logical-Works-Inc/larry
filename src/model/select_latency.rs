//! Route-choice latency with the pinned ranker on (larry#202): a measurement
//! control, not a seeded simulation, so it lives outside the `sim_*` modules
//! whose traces must replay identically (a sim that reads the wall clock
//! cannot replay). One cold ranking, then 2 warm ones over two routes,
//! printing p50/p95/p99 and peak RSS. The winner is stable across every run;
//! the printed numbers are the proof SHIP.md and ADR 0131 cite. Two warm runs
//! is all the 5-minute hang guard (`.config/nextest.toml`) fits at about a
//! minute an inference on the shared box; wall-clock times feed only the
//! printed card, never an assertion, so a loaded host changes the numbers,
//! never the pass.

#[cfg(feature = "rerank")]
use super::sim_select::{Check, TENANT, open_policy, stored};
#[cfg(feature = "rerank")]
use super::{Method, select};
#[cfg(feature = "rerank")]
use crate::model::route::Table;
#[cfg(feature = "rerank")]
use crate::rerank::Scorer;

/// Route-choice latency with the pinned ranker on: one cold ranking, then 2
/// warm ones over two routes, printing p50/p95/p99 and peak RSS.
#[cfg(feature = "rerank")]
#[test]
fn ranked_choice_latency_with_the_pinned_ranker() -> Check {
    use std::io::Write as _;

    let table = Table::new(
        vec![
            stored("alpha", Some("CA"), "drafts quarterly reviews for the team")?,
            stored(
                "beta",
                Some("CA"),
                "migrates relational databases between hosts",
            )?,
        ],
        None,
    );
    let policy = open_policy()?;
    let query = "migrate the production database to a new host";
    let mut held = crate::rerank::local_scorer()?;
    let scorer: &mut dyn Scorer = &mut held;
    // Cold: the weights load on the first score.
    let cold = std::time::Instant::now();
    let first = select(
        TENANT,
        &table,
        None,
        None,
        Some(query),
        &policy,
        Some(&mut *scorer),
    )?;
    let cold_ms = cold.elapsed().as_secs_f64() * 1000.0;
    let winner = first.route().id().to_owned();
    assert_eq!(first.method, Method::Ranked, "two routes rank");
    let mut millis = Vec::with_capacity(2);
    for round in 0..2 {
        let start = std::time::Instant::now();
        let choice = select(
            TENANT,
            &table,
            None,
            None,
            Some(query),
            &policy,
            Some(&mut *scorer),
        )?;
        millis.push(start.elapsed().as_secs_f64() * 1000.0);
        assert_eq!(
            choice.route().id(),
            winner,
            "the ranker is stable across runs"
        );
        let progress = format!("rank run {round} done\n");
        std::io::stderr().lock().write_all(progress.as_bytes())?;
    }
    millis.sort_by(f64::total_cmp);
    // Nearest-rank percentiles over the 2 warm runs.
    let nearest = |per_cent: usize| {
        let rank = millis.len().saturating_mul(per_cent).div_ceil(100);
        match millis.get(rank.saturating_sub(1)) {
            Some(sample) => *sample,
            None => 0.0,
        }
    };
    let rss_kib = std::fs::read_to_string("/proc/self/status")
        .ok()
        .and_then(|status| {
            status.lines().find_map(|line| {
                line.strip_prefix("VmHWM:")?
                    .split_whitespace()
                    .next()?
                    .parse::<u64>()
                    .ok()
            })
        });
    let rss = match rss_kib {
        Some(kib) => format!("{kib} KiB"),
        None => String::from("unknown"),
    };
    let line = format!(
        "route-choice with the reranker on (2 warm runs over 2 routes, winner {winner}): cold {cold_ms:.1} ms, p50 {:.1} ms, p95 {:.1} ms, p99 {:.1} ms, peak RSS {rss}\n",
        nearest(50),
        nearest(95),
        nearest(99),
    );
    std::io::stderr().lock().write_all(line.as_bytes())?;
    Ok(())
}
