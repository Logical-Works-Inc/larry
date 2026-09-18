"""Real fixture-driver subprocess journeys; run after cargo build --locked."""
import hashlib
import json
import os
from pathlib import Path
import select
import subprocess
import unittest

ROOT = Path(__file__).resolve().parents[1]
BINARY = ROOT / "target/debug/rocco-demo"


def run(data=b"", args=()):
    return subprocess.run([str(BINARY), *args], input=data, capture_output=True,
                          cwd=ROOT, timeout=5, check=False)


def frames(result):
    return [json.loads(line) for line in result.stdout.splitlines()]


class DemoJourneys(unittest.TestCase):
    def test_inspect_feedback_independent_work_and_continue(self):
        result = run(b"next\nfeedback bound retries\nnext\ninspect 2\nnext\ncontinue\nnext\nquit\n")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn(b"HeldWork", result.stderr)
        views = frames(result)
        self.assertEqual(views[4]["work"]["Held"]["anchor"], 1)
        self.assertEqual(views[4]["inspection"]["trace"], "fixture-02")
        self.assertEqual(views[-1]["inspection"]["line"], 18)
        self.assertEqual(views[-1]["feedback"][0]["text"], "bound retries")
        self.assertTrue(all(v["mode"].startswith("DEMO") for v in views))

    def test_panes_empty_scenario_and_invalid_operation_recovery(self):
        result = run(b"folders\nagents\nscenario empty\nnext\nexec touch rocco-unwanted\nscenario change\nnext\nquit\n")
        self.assertEqual(result.returncode, 0, result.stderr)
        views = frames(result)
        self.assertEqual(len(views[1]["folders"]), 2)
        self.assertEqual(views[2]["agents"][2]["parent"], 0)
        self.assertEqual(views[3]["scenario"], "empty")
        self.assertIn(b"EndOfFixture", result.stderr)
        self.assertIn(b"UnknownCommand", result.stderr)
        self.assertFalse((ROOT / "rocco-unwanted").exists())
        self.assertEqual(len(views[-1]["events"]), 1)

    def test_transport_bounds_utf8_eof_and_arguments(self):
        for data, args in [(b"x" * 4097 + b"\n", ()), (b"\xff\n", ()), (b"", ("--execute",))]:
            with self.subTest(data_length=len(data), args=args):
                result = run(data, args)
                self.assertEqual(result.returncode, 1)
                self.assertIn(b"DEMO failed", result.stderr)
        result = run(b"help\r\nstatus")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(len(frames(result)), 3)

    @unittest.skipUnless(os.name == "posix", "actual kill/select journey requires POSIX")
    def test_actual_process_crash_resets_fixture_without_workspace_changes(self):
        paths = list((ROOT / "src").glob("*.rs")) + [ROOT / "Cargo.toml", ROOT / "Cargo.lock"]
        before = {p: hashlib.sha256(p.read_bytes()).hexdigest() for p in paths}
        with subprocess.Popen([str(BINARY)], stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                              stderr=subprocess.PIPE, cwd=ROOT, bufsize=0) as child:
            try:
                assert child.stdin is not None and child.stdout is not None
                child.stdin.write(b"next\nfeedback preserve original anchor\n")
                snapshots = []
                for _ in range(3):
                    self.assertTrue(select.select([child.stdout], [], [], 5)[0], "driver stalled")
                    snapshots.append(json.loads(child.stdout.readline()))
                self.assertEqual(snapshots[-1]["work"]["Held"]["anchor"], 1)
                child.kill()
                self.assertEqual(child.wait(timeout=5), -9)
            finally:
                if child.poll() is None:
                    child.kill()
                    child.wait(timeout=5)
        restored = run(b"status\nquit\n")
        self.assertEqual(restored.returncode, 0, restored.stderr)
        self.assertEqual(frames(restored)[0]["work"], "Ready")
        self.assertEqual(frames(restored)[0]["feedback"], [])
        self.assertEqual(before, {p: hashlib.sha256(p.read_bytes()).hexdigest() for p in paths})


if __name__ == "__main__":
    unittest.main(verbosity=2)
