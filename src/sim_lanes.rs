//! Bounded scoped lanes for simulations (test builds only): a seed range split into contiguous
//! chunks, one scoped thread per chunk, the outputs joined back in seed order. Only `std` is
//! used, so a trace hash taken over the result never depends on which lane ran first, and every
//! thread is joined before the call returns. The same file is included by path into the
//! integration test binaries that sweep seeds (`tests/sim_exec_gate.rs`,
//! `tests/sim_model_client.rs`), so one owner serves every sweep.

use std::any::Any;
use std::thread;

use lgwks_macros::Error;

/// Most lanes one sweep ever runs, whatever the caller asks for.
pub(crate) const MAX_LANES: u64 = 16;

/// Most lanes the default sweep takes from the machine.
const MACHINE_LANES: u64 = 8;

/// Why a sweep gave no outputs.
#[derive(Debug, PartialEq, Eq, Error)]
pub(crate) enum LaneError {
    /// A sweep was asked to run on no lane at all.
    #[error("a sweep needs at least one lane")]
    NoLanes,
    /// A lane's work panicked, most often a failed assertion; its message is kept.
    #[error("lane {lane} panicked: {message}")]
    Panicked {
        /// The lane that panicked.
        lane: u64,
        /// What the panic said.
        message: String,
    },
}

/// The lanes this machine offers a sweep: its available parallelism, between one and
/// [`MACHINE_LANES`]. A machine that cannot say, or one core, runs the sweep serially with the
/// same result.
pub(crate) fn lanes() -> u64 {
    let cores = match thread::available_parallelism() {
        Ok(cores) => cores.get(),
        Err(_) => 1,
    };
    match u64::try_from(cores) {
        Ok(cores) => cores.clamp(1, MACHINE_LANES),
        Err(_) => 1,
    }
}

/// `work(lane, seed)` for every seed in `0..count` on at most [`lanes`] scoped threads; the
/// outputs are in seed order. A caller that keeps per-lane state sizes it with [`lanes`], the
/// count this sweep runs on.
pub(crate) fn sweep<Out: Send>(
    count: u64,
    work: impl Fn(u64, u64) -> Out + Sync,
) -> Result<Vec<Out>, LaneError> {
    sweep_on(lanes(), count, work)
}

/// The text a panic carried, or a fixed line when it carried none. This file is included by
/// path into integration test binaries too, so it cannot call the library's one reader.
fn message_of(cause: &(dyn Any + Send)) -> String {
    if let Some(text) = cause.downcast_ref::<String>() {
        return text.clone();
    }
    if let Some(text) = cause.downcast_ref::<&str>() {
        return String::from(*text);
    }
    String::from("a lane panicked without a message")
}

/// Runs `count` seeds over at most `lanes` scoped threads (never more
/// than [`MAX_LANES`]): lane `n` runs one contiguous chunk, in order, so the lane of a seed is
/// fixed by `lanes` and `count` alone. The outputs are in seed order. When a lane panics the
/// others still finish and are joined, and the first panicked lane is named. A sweep that keeps
/// a core busy passes one lane: nextest already runs one test per core, so lanes there only
/// oversubscribe the host (on the 4-core CI runner, run 43949143055, the CPU-bound sims took 1.4
/// to 3.8 times longer on lanes and slowed the tests beside them). Lanes pay on a sweep that
/// mostly waits ([`sweep`]).
pub(crate) fn sweep_on<Out: Send>(
    lanes: u64,
    count: u64,
    work: impl Fn(u64, u64) -> Out + Sync,
) -> Result<Vec<Out>, LaneError> {
    if lanes == 0 {
        return Err(LaneError::NoLanes);
    }
    let lanes = lanes.min(MAX_LANES);
    let chunk = count.div_ceil(lanes);
    let work = &work;
    let joined: Vec<(u64, thread::Result<Vec<Out>>)> = thread::scope(|scope| {
        let mut running = Vec::new();
        for lane in 0..lanes {
            let start = lane.saturating_mul(chunk);
            let end = start.saturating_add(chunk).min(count);
            if start >= end {
                break;
            }
            running.push((
                lane,
                scope.spawn(move || {
                    (start..end)
                        .map(|seed| work(lane, seed))
                        .collect::<Vec<Out>>()
                }),
            ));
        }
        running
            .into_iter()
            .map(|(lane, handle)| (lane, handle.join()))
            .collect()
    });
    let mut merged = Vec::new();
    let mut failure = None;
    for (lane, outcome) in joined {
        match outcome {
            Ok(outputs) => merged.extend(outputs),
            Err(cause) => {
                failure.get_or_insert(LaneError::Panicked {
                    lane,
                    message: message_of(cause.as_ref()),
                });
            }
        }
    }
    match failure {
        Some(error) => Err(error),
        None => Ok(merged),
    }
}
