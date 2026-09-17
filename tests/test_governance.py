"""Negative controls mutate text in memory, never repository files."""

from pathlib import Path
import json
import re
import signal
import subprocess
import sys
import tempfile
import unittest

from tools.check_governance import load_documents, receipt_errors, validate


ROOT = Path(__file__).resolve().parents[1]


class GovernanceTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.documents = load_documents(ROOT)

    def reject(self, name, old, new, reason):
        docs = dict(self.documents)
        self.assertIn(old, docs[name])
        docs[name] = docs[name].replace(old, new, 1)
        result = validate(docs)
        self.assertFalse(result["ok"], result)
        self.assertTrue(any(reason in error for error in result["errors"]), result)

    def test_current_documents(self):
        report = validate(self.documents)
        self.assertTrue(report["ok"], report["errors"])
        self.assertGreaterEqual(report["counts"]["requirements"], 70)
        self.assertGreaterEqual(len(report["planned_crash_cases"]) * 10, report["counts"]["falsifiers"])

    def test_missing_document(self):
        docs = dict(self.documents)
        docs.pop("AGENTS.md")
        self.assertIn("missing document: AGENTS.md", validate(docs)["errors"])

    def test_anchor_case_does_not_change_meaning(self):
        docs = dict(self.documents)
        docs["AGENTS.md"] = docs["AGENTS.md"].replace("Memory", "MEMORY")
        self.assertTrue(validate(docs)["ok"])

    def test_missing_owner_anchor(self):
        self.reject("AGENTS.md", "Sentinel", "unnamed owner", "missing governance anchor Sentinel")

    def test_removed_requirement(self):
        self.reject("specification.md", "| R01 |", "| removed |", "requirements:")

    def test_duplicate_requirement(self):
        self.reject("specification.md", "| R02 |", "| R01 |", "requirements:")

    def test_undefined_decision(self):
        self.reject("specification.md", "| D01 | E01 |", "| D99 | E01 |", "undefined D")

    def test_missing_acceptance_mapping(self):
        self.reject("specification.md", "| D01 | E01 |", "| D01 | none |", "unmapped requirement")

    def test_stale_inventory(self):
        count = validate(self.documents)["counts"]["requirements"]
        self.reject("log.md", f"requirements: {count}", f"requirements: {count - 1}", "log inventory drift")

    def test_missing_execution_case_allocation(self):
        self.reject("specification.md", "C05 owns E06/E36", "C05 owns E36", "unallocated execution cases")

    def test_unallocated_mention_is_not_assignment(self):
        docs = dict(self.documents)
        docs["specification.md"] = docs["specification.md"].replace("C05 owns E06/E36", "C05 owns E36")
        docs["specification.md"] = docs["specification.md"].replace("## Cell 12", "Unallocated case: E06.\n\n## Cell 12")
        self.assertTrue(any("unallocated execution cases" in error for error in validate(docs)["errors"]))

    def test_insufficient_crash_cases(self):
        line = re.findall(r"^planned_crash_cases: .+$", self.documents["log.md"], re.M)[0]
        self.reject("log.md", line,
                    "planned_crash_cases: E02", "below 10 percent")

    def test_duplicate_crash_case(self):
        self.reject("log.md", "planned_crash_cases: E02 E13", "planned_crash_cases: E02 E02",
                    "invalid planned crash")

    def test_skill_is_not_a_kill(self):
        line = re.findall(r"^planned_crash_cases: .+$", self.documents["log.md"], re.M)[0]
        self.reject("log.md", line,
                    "planned_crash_cases: E02 E13 E19 E28 E17 E26", "lacks fault action")

    def test_inventory_can_grow(self):
        docs = dict(self.documents)
        count = validate(docs)["counts"]["requirements"]
        for next_count in (count + 1, count + 2):
            docs["specification.md"] += f"\n| R{next_count:02d} | extension | Additional mapped requirement | D25 | E28 |\n"
            docs["log.md"] = re.sub(r"^requirements: \d+$", f"requirements: {next_count}", docs["log.md"], flags=re.M)
            report = validate(docs)
            self.assertTrue(report["ok"], report["errors"])
            self.assertEqual(report["counts"]["requirements"], next_count)

    def test_missing_frontmatter(self):
        self.reject("index.md", "---\n", "", "missing frontmatter")

    def test_duplicate_frontmatter(self):
        self.reject("index.md", "type: Index", "type: Index\ntype: Index", "duplicate frontmatter")

    def test_unclosed_fence(self):
        docs = dict(self.documents)
        docs["specification.md"] += "\n```mermaid\nflowchart TD\n"
        self.assertIn("specification.md: unclosed code fence", validate(docs)["errors"])

    def test_missing_visuals(self):
        docs = dict(self.documents)
        docs["specification.md"] = docs["specification.md"].replace("```mermaid", "```text")
        self.assertIn("specification lacks required visual diagrams", validate(docs)["errors"])

    def test_valid_tilde_mermaid_fences(self):
        docs = dict(self.documents)
        docs["specification.md"] = docs["specification.md"].replace("```", "~~~")
        self.assertTrue(validate(docs)["ok"])

    def test_unclosed_indented_and_tilde_fences(self):
        for opener in ("~~~python", "  ```python", "   ````text"):
            with self.subTest(opener=opener):
                docs = dict(self.documents)
                docs["index.md"] += "\n" + opener + "\nunterminated\n"
                self.assertIn("index.md: unclosed code fence", validate(docs)["errors"])

    def test_matching_long_fence_with_short_content_fence(self):
        docs = dict(self.documents)
        docs["index.md"] += "\n````text\n```\ncontent\n````\n"
        self.assertTrue(validate(docs)["ok"])

    def test_escaping_link(self):
        self.reject("README.md", "](index.md)", "](../index.md)", "escaping local link")

    def test_missing_link(self):
        self.reject("README.md", "](index.md)", "](missing.md)", "unresolved")

    def test_missing_heading_fragment(self):
        self.reject("README.md", "](index.md)", "](index.md#definitely-missing)", "unresolved heading fragment")

    def test_missing_same_document_fragment(self):
        self.reject("README.md", "](index.md)", "](#definitely-missing)", "unresolved heading fragment")

    def test_valid_heading_fragment(self):
        docs = dict(self.documents)
        docs["README.md"] = docs["README.md"].replace("](index.md)", "](index.md#read-in-this-order)")
        self.assertTrue(validate(docs)["ok"])

    def test_code_block_heading_is_not_destination(self):
        docs = dict(self.documents)
        docs["index.md"] += "\n```text\n# Phantom heading\n```\n"
        docs["README.md"] = docs["README.md"].replace("](index.md)", "](index.md#phantom-heading)")
        self.assertTrue(any("unresolved heading fragment" in error for error in validate(docs)["errors"]))

    def test_unsafe_link_scheme(self):
        self.reject("README.md", "](index.md)", "](javascript:bad)", "unsupported link scheme")

    def test_oversized_document(self):
        from tools.check_governance import MAX_DOCUMENT_BYTES
        docs = dict(self.documents)
        docs["index.md"] += "x" * MAX_DOCUMENT_BYTES
        self.assertIn("index.md: document exceeds byte limit", validate(docs)["errors"])

    def test_cli_success(self):
        result = subprocess.run([sys.executable, "tools/check_governance.py"],
                                cwd=ROOT, capture_output=True, text=True, timeout=10)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("PASS: document structure", result.stdout)
        self.assertIn("NOT RUN:", result.stdout)

    def test_cli_failure(self):
        result = subprocess.run([sys.executable, "tools/check_governance.py", "--root", str(ROOT / "tests")],
                                cwd=ROOT, capture_output=True, text=True, timeout=10)
        self.assertEqual(result.returncode, 1, result.stdout)
        self.assertIn("FAIL: missing document", result.stderr)
        self.assertNotIn("PASS:", result.stdout)

    def test_cli_json_report(self):
        result = subprocess.run([sys.executable, "tools/check_governance.py", "--report"],
                                cwd=ROOT, capture_output=True, text=True, timeout=10)
        self.assertEqual(result.returncode, 0, result.stderr)
        report = json.loads(result.stdout)
        self.assertTrue(report["ok"])
        self.assertEqual(set(report["sha256"]), set(self.documents))

    def test_reranker_receipts_are_present_and_valid(self):
        self.assertEqual(receipt_errors(ROOT), [])

    def test_missing_reranker_receipt_is_rejected(self):
        with tempfile.TemporaryDirectory() as temporary:
            self.assertTrue(any("none retained" in error for error in receipt_errors(Path(temporary))))

    def test_malformed_reranker_receipt_is_rejected(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            path = root / "tools" / "receipts"
            path.mkdir(parents=True)
            (path / "pre.json").write_text(json.dumps({
                "phase": "pre-exec", "model": "Qwen/Qwen3-Reranker-0.6B",
                "revision": "e61197ed45024b0ed8a2d74b80b4d909f1255473",
                "query_sha256": "x" * 64, "candidates_sha256": "y" * 64,
                "candidate_count": 1, "top_k": 1, "results": ["forged"],
            }))
            self.assertTrue(any("invalid bounds or hashes" in error for error in receipt_errors(root)))

    def test_docs_tree_is_admitted_and_validated(self):
        self.assertIn("docs/README.md", self.documents)
        self.assertTrue(validate(self.documents)["ok"])

    def test_docs_tree_frontmatter_is_enforced(self):
        docs = dict(self.documents)
        docs["docs/dependencies/README.md"] = docs["docs/dependencies/README.md"].replace(
            "status:", "statuz:", 1)
        result = validate(docs)
        self.assertFalse(result["ok"], result)
        self.assertTrue(any("docs/dependencies/README.md" in error
                            and "missing frontmatter keys" in error
                            for error in result["errors"]), result)

    def test_docs_tree_undefined_reference_is_rejected(self):
        docs = dict(self.documents)
        docs["docs/README.md"] += "\nStray reference R99 must fail traceability.\n"
        self.assertTrue(any("docs/README.md: undefined R references ['R99']" in error
                            for error in validate(docs)["errors"]))

    def test_docs_tree_escaping_link_is_rejected(self):
        docs = dict(self.documents)
        self.assertIn("](/docs/architecture/README.md)", docs["docs/README.md"])
        docs["docs/README.md"] = docs["docs/README.md"].replace(
            "](/docs/architecture/README.md)", "](../docs/architecture/README.md)", 1)
        self.assertTrue(any("docs/README.md: unresolved or escaping local link" in error
                            for error in validate(docs)["errors"]))

    @unittest.skipIf(sys.platform == "win32", "POSIX process-crash control; Windows remains separate")
    def test_owned_checker_crash_and_recovery(self):
        before = validate(load_documents(ROOT))["sha256"]
        code = ("import os, signal; from pathlib import Path; "
                "from tools.check_governance import load_documents, validate; "
                "validate(load_documents(Path.cwd())); "
                "os.kill(os.getpid(), signal.SIGKILL)")
        crashed = subprocess.run([sys.executable, "-c", code], cwd=ROOT,
                                 capture_output=True, text=True, timeout=10)
        self.assertEqual(crashed.returncode, -signal.SIGKILL)
        recovered = subprocess.run([sys.executable, "tools/check_governance.py", "--report"],
                                   cwd=ROOT, capture_output=True, text=True, timeout=10)
        self.assertEqual(recovered.returncode, 0, recovered.stderr)
        self.assertEqual(json.loads(recovered.stdout)["sha256"], before)


if __name__ == "__main__":
    unittest.main()
