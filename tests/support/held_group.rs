//! Fixtures for the journeys that kill the shipped `larry exec` while its acceptance
//! command's process group is running (larry#85, larry#214): the held command, the probe
//! that waits for a group to end, and the ledger's row count. `tests/executor_journey.rs`
//! and `tests/sim_orphan_groups.rs` call every helper here.

use std::error::Error;
use std::fs::File;
use std::path::Path;
use std::process::Child;

use lgwks_deps::storefront::rusqlite::{Connection, OpenFlags};

use crate::exec_cli::{POLL_LIMIT, live_group_pid, make_fifo, spawn_exec, task_json};

/// Starts `larry exec` on a task whose one acceptance command holds its process group
/// open on a fifo under `scratch`, and returns the executor with that group's pid once
/// the group is recorded and running. The `ready` reader comes back open too: dropping it
/// early would let the command's first write die of `SIGPIPE` and end the group.
pub fn spawn_held_command(
    state: &Path,
    origin: &Path,
    scratch: &Path,
) -> Result<(Child, i32, File), Box<dyn Error>> {
    let ready = scratch.join("ready");
    let hold = scratch.join("hold");
    make_fifo(&ready)?;
    make_fifo(&hold)?;
    let acceptance = format!(
        r#"[["sh","-c","echo up > {}; cat < {}"]]"#,
        ready.display(),
        hold.display()
    );
    let task = task_json(origin, "HEAD", &acceptance);
    let child = spawn_exec(Some(state), None, &task)?;
    // This open blocks until the acceptance command opens the fifo, so the command is running.
    let reader = File::open(&ready)?;
    let pid = live_group_pid(&state.join("executor.db"))?;
    Ok((child, pid, reader))
}

/// Polls until process group `pgid` is gone, at most `POLL_LIMIT` probes; killed members
/// may linger briefly as zombies. `false` when the group outlives every probe.
pub fn group_exits(pgid: i32) -> Result<bool, Box<dyn Error>> {
    let mut attempts: u32 = 0;
    while lgwks_std::process::process_group_exists(pgid)? {
        attempts = attempts.saturating_add(1);
        if attempts >= POLL_LIMIT {
            return Ok(false);
        }
        std::thread::yield_now();
    }
    Ok(true)
}

/// The number of rows the executor's ledger holds, read through a read-only open.
pub fn ledger_rows(db: &Path) -> Result<i64, Box<dyn Error>> {
    let connection = Connection::open_with_flags(db, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    Ok(connection.query_row("SELECT COUNT(*) FROM live_groups", [], |row| row.get(0))?)
}
