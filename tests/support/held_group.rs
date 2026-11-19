//! Fixtures for the journeys that kill the shipped `larry exec` while its acceptance
//! command's process group is running (larry#85, larry#214): the held command and the
//! ledger's row count. `tests/executor_journey.rs` calls every helper here, so a held group
//! is started and read one way.

use std::error::Error;
use std::fs::File;
use std::path::Path;
use std::process::Child;

use lgwks_deps::storefront::rusqlite::{Connection, OpenFlags};

use crate::exec_cli::deadline::await_fifo_writer;
use crate::exec_cli::{live_group_pid, make_fifo, spawn_exec, task_json};

/// Starts `larry exec` on a task whose one acceptance command holds its process group open on
/// a fifo under `scratch`, and returns the executor, that group's pid, and the fifo's write
/// handle. The command is already blocked on the read end when this returns, and the handle
/// must stay alive: it is the reader that keeps the command running.
///
/// The open of the write end blocks until the command opens the read end, so the pipe is the
/// event that says the command started; a command that never starts fails that wait inside
/// one deadline instead of hanging the journey (larry#268).
pub(crate) fn spawn_held_command(
    state: &Path,
    origin: &Path,
    scratch: &Path,
) -> Result<(Child, i32, File), Box<dyn Error>> {
    let hold = scratch.join("hold");
    make_fifo(&hold)?;
    let acceptance = format!(r#"[["sh","-c","cat < {}"]]"#, hold.display());
    let task = task_json(origin, "HEAD", &acceptance);
    let child = spawn_exec(Some(state), None, &task)?;
    let writer = await_fifo_writer(&hold)?;
    let pid = live_group_pid(&state.join("executor.db"))?;
    Ok((child, pid, writer))
}

/// The number of rows the executor's ledger holds, read through a read-only open.
pub(crate) fn ledger_rows(db: &Path) -> Result<i64, Box<dyn Error>> {
    let connection = Connection::open_with_flags(db, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    Ok(connection.query_row("SELECT COUNT(*) FROM live_groups", [], |row| row.get(0))?)
}
