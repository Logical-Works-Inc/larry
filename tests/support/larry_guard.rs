// The one guard over a `larry` process a test starts (larry#573): the child is killed and reaped
// when the guard drops, on an early return or an unwind too, so no test leaves a `larry` behind.
// Thirty-four orphans held four cores of the build box before this guard. Every spawn helper
// returns it; each test crate declares it once at its root, beside `coverage`.

use std::io::{self, Read};
use std::ops::{Deref, DerefMut};
use std::process::{Child, Command, Output};

/// A started `larry`, owned by the test that started it.
#[derive(Debug)]
pub(crate) struct Larry(Child);

impl Larry {
    /// Starts `command` and takes ownership of the child.
    pub(crate) fn spawn(command: &mut Command) -> io::Result<Self> {
        command.spawn().map(Self)
    }

    /// Closes stdin, reads both pipes to their end at once and waits for the exit, as
    /// `Child::wait_with_output` does, while the guard still owns the child.
    pub(crate) fn wait_with_output(mut self) -> io::Result<Output> {
        self.0.stdin = None;
        let out = self.0.stdout.take();
        let err = self.0.stderr.take();
        let (stdout, stderr) = std::thread::scope(|scope| {
            let reader = scope.spawn(move || read_to_end(err));
            let stdout = read_to_end(out);
            let stderr = match reader.join() {
                Ok(read) => read,
                Err(panic) => Err(io::Error::other(format!(
                    "the stderr reader panicked: {panic:?}"
                ))),
            };
            (stdout, stderr)
        });
        Ok(Output {
            status: self.0.wait()?,
            stdout: stdout?,
            stderr: stderr?,
        })
    }
}

// Each test crate compiles this file on its own and some never collect output. The lint floor
// forbids `allow`, so this reference keeps the method live there and pins its `Child` shape.
const _: fn(Larry) -> io::Result<Output> = Larry::wait_with_output;

/// Every byte of `pipe`; none when the stream was not piped.
fn read_to_end(pipe: Option<impl Read>) -> io::Result<Vec<u8>> {
    let mut bytes = Vec::new();
    if let Some(mut stream) = pipe {
        stream.read_to_end(&mut bytes)?;
    }
    Ok(bytes)
}

impl Deref for Larry {
    type Target = Child;

    fn deref(&self) -> &Child {
        &self.0
    }
}

impl DerefMut for Larry {
    fn deref_mut(&mut self) -> &mut Child {
        &mut self.0
    }
}

impl Drop for Larry {
    fn drop(&mut self) {
        // A child already reaped is left alone: its pid may name another process by now.
        let reaped = match self.0.try_wait() {
            Ok(Some(_status)) => Ok(()),
            Ok(None) => self.0.kill().and_then(|()| self.0.wait().map(|_status| ())),
            Err(cause) => Err(cause),
        };
        // A child that would not die fails the test that owned it, unless it is already failing.
        assert!(
            std::thread::panicking() || reaped.is_ok(),
            "the larry child is killed and reaped: {reaped:?}"
        );
    }
}
