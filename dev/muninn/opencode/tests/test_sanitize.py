"""Tests for the redaction rules."""

from __future__ import annotations

import unittest

from membench.sanitize import redact_text, sanitize


class SanitizeTests(unittest.TestCase):
    def test_secret_keys_are_redacted(self) -> None:
        payload = {"api_key": "abc123", "nested": {"password": "hunter2"}}
        result = sanitize(payload)
        self.assertEqual(result["api_key"], "[REDACTED]")
        self.assertEqual(result["nested"]["password"], "[REDACTED]")

    def test_embeddings_are_omitted(self) -> None:
        payload = {"embedding": [0.1, 0.2, 0.3], "content": "keep"}
        result = sanitize(payload)
        self.assertEqual(result["embedding"], "[EMBEDDING OMITTED]")
        self.assertEqual(result["content"], "keep")

    def test_bearer_tokens_are_removed_from_text(self) -> None:
        cleaned = redact_text("Authorization: Bearer abcdef123456")
        self.assertNotIn("abcdef123456", cleaned)

    def test_key_shaped_tokens_are_removed(self) -> None:
        cleaned = redact_text("token sk-abcdefgh12345678")
        self.assertNotIn("sk-abcdefgh12345678", cleaned)

    def test_long_text_is_truncated(self) -> None:
        cleaned = redact_text("x" * 5000, max_length=100)
        self.assertTrue(cleaned.endswith("chars]"))
        self.assertLess(len(cleaned), 200)

    def test_lists_are_sanitized_recursively(self) -> None:
        result = sanitize([{"secret": "value"}, "plain"])
        self.assertEqual(result[0]["secret"], "[REDACTED]")
        self.assertEqual(result[1], "plain")


if __name__ == "__main__":
    unittest.main()
