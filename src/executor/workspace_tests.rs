//! Unit tests of the origin lock (larry#214): a held lock keeps the origin, a
//! released one lets the sweep remove it with its stale lock file.

use super::{hold_origin, origin_held, sweep_origins};

/// A pid no live process holds: the largest positive `i32` less one.
const DEAD_PID: i32 = 2_147_483_646;

/// An origin whose pid is dead but whose lock a live eval holds is kept, and removed
/// with its lock file once the lock is released (larry#214).
#[test]
fn an_eval_origin_is_live_while_its_lock_is_held() -> Result<(), Box<dyn std::error::Error>> {
    let state = crate::executor::sim_tree::scratch("larry-origin-lock")?;
    let eval = state.join("eval");
    let name = format!("origin-{DEAD_PID}-0");
    let origin = eval.join(&name);
    std::fs::create_dir_all(&origin)?;
    let held = hold_origin(&eval, &name)?;
    assert!(
        origin_held(&eval, &name)?,
        "a held lock reads as live, whatever pid the name carries"
    );
    sweep_origins(&state)?;
    assert!(
        origin.is_dir(),
        "the sweep keeps an origin whose lock is held"
    );
    drop(held);
    assert!(!origin_held(&eval, &name)?, "a released lock reads as dead");
    sweep_origins(&state)?;
    assert!(
        !origin.exists(),
        "the sweep removes an origin once its lock is released"
    );
    assert!(
        !eval.join(format!("{name}.lock")).exists(),
        "the sweep removes the stale lock file"
    );
    Ok(())
}
