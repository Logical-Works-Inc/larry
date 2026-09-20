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
# Runaway-output bound. A 2026-09-19 8fps frame-loop trial crossed this limit
# in under 30 seconds on the same PTY and was removed; the input-driven build
# is remeasured in docs/evidence/workspace-final-metrics.json. A crash closes
# the PTY and fails distinctly above rather than being mistaken for quietness.
OUTPUT_BOUND = 1024 * 1024


class Terminal:
    def __init__(self):
        self.master, self.slave = pty.openpty()
        fcntl.ioctl(self.slave, termios.TIOCSWINSZ, struct.pack("HHHH", 32, 120, 0, 0))
        self.child = subprocess.Popen([str(BINARY)], stdin=self.slave, stdout=self.slave,
                                      stderr=self.slave, cwd=ROOT,
                                      env=os.environ | {"TERM": "xterm-256color"})
        self.output = bytearray()
        self.pending = bytearray()
        self.cursor_x = 0
        self.cursor_y = 0
        self.screen = [[" "] * 120 for _ in range(32)]

    def _move_cursor(self, params):
        fields = params.split(b";")
        row = int(fields[0] or b"1") if fields else 1
        column = int(fields[1] or b"1") if len(fields) > 1 else 1
        self.cursor_y = max(0, min(len(self.screen) - 1, row - 1))
        self.cursor_x = max(0, min(len(self.screen[0]) - 1, column - 1))

    def _write_screen(self, character):
        if character.isprintable() and self.cursor_y < len(self.screen):
            self.screen[self.cursor_y][self.cursor_x] = character
        self.cursor_x += 1
        if self.cursor_x >= len(self.screen[0]):
            self.cursor_x = 0
            self.cursor_y = min(len(self.screen) - 1, self.cursor_y + 1)

    def _ingest(self, chunk):
        self.pending.extend(chunk)
        index = 0
        while index < len(self.pending):
            current = self.pending[index]
            if current == 0x1B:
                if index + 1 >= len(self.pending):
                    break
                kind = self.pending[index + 1]
                if kind == ord("["):
                    final = index + 2
                    while final < len(self.pending) and not 0x40 <= self.pending[final] <= 0x7E:
                        final += 1
                    if final >= len(self.pending):
                        break
                    params = bytes(self.pending[index + 2:final])
                    command = self.pending[final]
                    if command in (ord("H"), ord("f")):
                        self._move_cursor(params)
                    elif command == ord("J"):
                        self.screen = [[" "] * 120 for _ in range(32)]
                    index = final + 1
                    continue
                if kind == ord("]"):
                    final = index + 2
                    while final < len(self.pending):
                        if self.pending[final] == 0x07:
                            final += 1
                            break
                        if self.pending[final:final + 2] == b"\x1b\\":
                            final += 2
                            break
                        final += 1
                    if final > len(self.pending):
                        break
                    index = final
                    continue
            if current < 0x80:
                if current == ord("\r"):
                    self.cursor_x = 0
                elif current == ord("\n"):
                    self.cursor_y = min(len(self.screen) - 1, self.cursor_y + 1)
                elif current >= 0x20:
                    self._write_screen(chr(current))
                index += 1
                continue
            width = 2 if current & 0xE0 == 0xC0 else 3 if current & 0xF0 == 0xE0 else 4
            if index + width > len(self.pending):
                break
            self._write_screen(bytes(self.pending[index:index + width]).decode("utf-8", "replace"))
            index += width
        del self.pending[:index]

    def _visible_text(self):
        return "\n".join("".join(row) for row in self.screen)

    def expect(self, text):
        deadline = time.monotonic() + 5
        while text not in self._visible_text():
            remaining = deadline - time.monotonic()
            if remaining <= 0 or not select.select([self.master], [], [], remaining)[0]:
                raise AssertionError(f"missing {text!r}; screen={self._visible_text()!r}")
            chunk = os.read(self.master, 65536)
            if not chunk:
                raise AssertionError("terminal closed before expected output")
            self._ingest(chunk)
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
                chunk = os.read(self.master, 65536)
                self._ingest(chunk)
                self.output.extend(chunk)
                if len(self.output) > OUTPUT_BOUND:
                    raise AssertionError("native output bound exceeded")
        return self.child.wait(timeout=1)


class NativeJourneys(unittest.TestCase):
    def test_native_draw_input_panes_hold_continue_and_exit(self):
        terminal = Terminal()
        try:
            terminal.expect("Everything else stays quiet")
            terminal.send("\r")
            terminal.expect("Awaiting Moo")
            for command, expected in [
                ("/folders\r", "FILES / fixture"),
                ("/agents\r", "AGENTS / fixture"),
                ("/changes\r", "CHANGES / fixture"),
                ("/next\r", "src/retry.rs:12"),
                ("/inspect\r", "fixture-01"),
                ("/feedback keep bounded\r", "HELD clone 1"),
                ("/next\r", "fixture-02"),
                ("/next\r", "HeldWork"),
                ("/continue\r", "Awaiting Moo"),
                ("/fixture-continue\r", "no backend action executed"),
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
            terminal.expect("Everything else stays quiet")
            terminal.send("\r")
            terminal.expect("Awaiting Moo")
            terminal.send("/changes\r")
            terminal.expect("CHANGES / fixture")
            terminal.send("/next\r")
            terminal.expect("src/retry.rs:12")
            terminal.child.kill()
            self.assertEqual(terminal.child.wait(timeout=5), -signal.SIGKILL)
        finally:
            terminal.close()
        recovered = Terminal()
        try:
            recovered.expect("Everything else stays quiet")
            recovered.send("q")
            self.assertEqual(recovered.finish(), 0)
        finally:
            recovered.close()

    def test_board_truth_mouse_and_draft_recovery(self):
        terminal = Terminal()
        try:
            terminal.expect("Everything else stays quiet")
            terminal.send("t")
            terminal.expect("SYNTHETIC DATA")
            terminal.send("t")
            terminal.expect("Everything else stays quiet")
            # Real xterm SGR mouse input, 1-based coordinates inside rail row 3.
            terminal.send("\x1b[<0;8;17M\x1b[<0;8;17m")
            terminal.expect("Awaiting Moo")
            terminal.expect("Calmer project home")
            terminal.send("remember this")
            terminal.expect("remember this")
            terminal.send("\x1b")
            terminal.expect("Everything else stays quiet")
            terminal.send("2")
            terminal.expect("Workspace handoff")
            terminal.send("\x1b")
            terminal.expect("Everything else stays quiet")
            terminal.send("3")
            terminal.expect("remember this")
            terminal.send("\x15/quit\r")
            self.assertEqual(terminal.finish(), 0)
        finally:
            terminal.close()


if __name__ == "__main__":
    unittest.main(verbosity=2)
