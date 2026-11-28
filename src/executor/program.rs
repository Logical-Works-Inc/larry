//! Whether the host can start a command's program, asked before any wrapper stands in front of
//! it (larry#598, ADR 0145). Sources: execvp(3) and the PATH rules of IEEE Std 1003.1, 8.3.

use std::io;
// `run.rs`, which declares this module, refuses to build on a target that is not Unix.
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::path::Path;

/// Refuses `program` when the host cannot start it from `workspace`. A confined command is one
/// with a `scratch` directory of its own; a relative path in its tree names a file the model
/// writes, so it is not looked up and the wrapper's exit judges it.
pub(super) fn startable(
    program: &str,
    scratch: Option<&Path>,
    workspace: &Path,
    path_env: &str,
) -> io::Result<()> {
    let in_the_models_tree =
        scratch.is_some() && program.contains('/') && Path::new(program).is_relative();
    if in_the_models_tree {
        return Ok(());
    }
    looked_up(program, workspace, path_env)
}

/// The lookup execvp(3) would make: a name with a slash is that file; a bare name is the first
/// executable file of that name on `path_env`, an empty or relative entry read from the
/// workspace. A name found only as something that cannot run is refused permission, a name found
/// nowhere (the empty name always) is not found, and a PATH entry that cannot be read for any
/// other reason is that error.
fn looked_up(program: &str, workspace: &Path, path_env: &str) -> io::Result<()> {
    if program.contains('/') {
        return executable(&workspace.join(program));
    }
    let mut refusal = io::Error::new(
        io::ErrorKind::NotFound,
        "the command's PATH holds no such program",
    );
    // Joined to a PATH entry, the empty name would be the entry itself.
    if program.is_empty() {
        return Err(refusal);
    }
    for directory in std::env::split_paths(path_env) {
        let Err(error) = executable(&workspace.join(directory).join(program)) else {
            return Ok(());
        };
        match error.kind() {
            // execvp goes on to the next entry past these two.
            io::ErrorKind::NotFound | io::ErrorKind::NotADirectory => {}
            io::ErrorKind::PermissionDenied => refusal = error,
            _ => return Err(error),
        }
    }
    Err(refusal)
}

/// Whether `path` is a file the kernel would run: a regular file with an execute bit.
fn executable(path: &Path) -> io::Result<()> {
    let found = std::fs::metadata(path)?;
    if found.is_file() && found.permissions().mode() & 0o111 != 0 {
        return Ok(());
    }
    Err(io::Error::new(
        io::ErrorKind::PermissionDenied,
        "not an executable file",
    ))
}
