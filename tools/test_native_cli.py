"""Exercise the actual AppCUI binary in isolated POSIX pseudo-terminals."""
import fcntl
import os
from pathlib import Path
import pty
import re
import select
import signal
import struct
import subprocess
import termios
import time
import unittest

ROOT = Path(__file__).resolve().parents[1]
BINARY = ROOT / "target/debug/rocco"
ANSI = re.compile(rb"\x1b\[[0-?]*[ -/]*[@-~]|\x1b\][^\x07]*(?:\x07|\x1b\\)")
# Runaway-output bound. Measured 2026-09-18: the ten-command hold/Continue
# journey emits ~566 KiB total (~5.5 KiB per event-driven repaint, idle
# ~3 KiB/s), so 1 MiB leaves headroom while a spew loop still trips it
# within a second. A crash closes the PTY and fails distinctly above.
OUTPUT_BOUND = 1024 * 1024


class Terminal:
    def __init__(self):
        self.master, self.slave = pty.openpty()
        fcntl.ioctl(self.slave, termios.TIOCSWINSZ, struct.pack("HHHH", 32, 120, 0, 0))
        self.child = subprocess.Popen([str(BINARY)], stdin=self.slave, stdout=self.slave,
                                      stderr=self.slave, cwd=ROOT,
                                      env=os.environ | {"TERM": "xterm-256color"})
        self.output = bytearray()

    def expect(self, text):
        fragment = bytearray()
        deadline = time.monotonic() + 5
        while text.encode() not in ANSI.sub(b"", fragment):
            remaining = deadline - time.monotonic()
            if remaining <= 0 or not select.select([self.master], [], [], remaining)[0]:
                raise AssertionError(f"missing {text!r}; output={ANSI.sub(b'', fragment)!r}")
            chunk = os.read(self.master, 65536)
            if not chunk:
                raise AssertionError("terminal closed before expected output")
            fragment.extend(chunk)
            self.output.extend(chunk)
            if len(self.output) > OUTPUT_BOUND:
                raise AssertionError("native test output exceeded 1 MiB")

    def send(self, text):
        os.write(self.master, text.encode())

    def close(self):
        if self.child.poll() is None:
            self.child.kill()
        self.child.wait(timeout=5)
        os.close(self.master)
        os.close(self.slave)

    def finish(self):
        deadline = time.monotonic() + 5
        while self.child.poll() is None:
            remaining = deadline - time.monotonic()
            if remaining <= 0:
                raise AssertionError("native exit exceeded five seconds")
            if select.select([self.master], [], [], min(remaining, 0.05))[0]:
                self.output.extend(os.read(self.master, 65536))
                if len(self.output) > OUTPUT_BOUND:
                    raise AssertionError("native output bound exceeded")
        return self.child.wait(timeout=1)


class NativeJourneys(unittest.TestCase):
    def test_native_draw_input_panes_hold_continue_and_exit(self):
        terminal = Terminal()
        try:
            terminal.expect("Safe fixture")
            for command, expected in [
                ("/folders\r", "Files [fixture]"),
                ("/agents\r", "Agents [fixture]"),
                ("/changes\r", "▸ Changes"),
                ("/next\r", "src/retry.rs:12"),
                ("/inspect\r", "fixture-01"),
                ("/feedback keep bounded\r", "HELD clone 1"),
                ("/next\r", "fixture-02"),
                ("/next\r", "HeldWork"),
                ("/continue\r", "no backend action executed"),
                ("/next\r", "fixture-03"),
                ("/scenario empty\r", "No changes yet"),
            ]:
                terminal.send(command)
                terminal.expect(expected)
            terminal.send("/quit\r")
            self.assertEqual(terminal.finish(), 0)
        finally:
            terminal.close()

    def test_native_crash_and_fresh_terminal_recovery(self):
        terminal = Terminal()
        try:
            terminal.expect("Safe fixture")
            terminal.send("/changes\r")
            terminal.expect("▸ Changes")
            terminal.send("/next\r")
            terminal.expect("src/retry.rs:12")
            terminal.child.kill()
            self.assertEqual(terminal.child.wait(timeout=5), -signal.SIGKILL)
        finally:
            terminal.close()
        recovered = Terminal()
        try:
            recovered.expect("Safe fixture")
            recovered.send("/quit\r")
            self.assertEqual(recovered.finish(), 0)
        finally:
            recovered.close()


if __name__ == "__main__":
    unittest.main(verbosity=2)
