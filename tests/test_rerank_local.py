import io
import unittest
from unittest.mock import patch

import tools.rerank_local as rerank_local


class RerankerContractTests(unittest.TestCase):
    def rows(self, text):
        with patch("sys.stdin", io.StringIO(text)):
            return rerank_local.candidates(None)

    def test_accepts_unique_nonempty_candidates(self):
        rows = self.rows('{"id":"a","text":"alpha"}\n{"id":"b","text":"beta"}\n')
        self.assertEqual(rows, [{"id": "a", "text": "alpha"}, {"id": "b", "text": "beta"}])

    def test_rejects_duplicate_ids(self):
        with self.assertRaisesRegex(ValueError, "unique"):
            self.rows('{"id":"a","text":"alpha"}\n{"id":"a","text":"beta"}\n')

    def test_rejects_missing_text(self):
        with self.assertRaisesRegex(ValueError, "non-empty"):
            self.rows('{"id":"a","text":""}\n')

    def test_rejects_non_object_candidate(self):
        with self.assertRaisesRegex(ValueError, "JSON object"):
            self.rows('"not an object"\n')
