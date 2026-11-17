import os
from pathlib import Path
import unittest

from minilm_tokenizer_reference import generate


class ReferenceTests(unittest.TestCase):
    @unittest.skipUnless(os.environ.get("MUNINN_MINILM_BUNDLE"), "Requires the explicit pinned local bundle")
    def test_reference_encodes_empty_text_with_exact_specials(self):
        result = generate(Path(os.environ["MUNINN_MINILM_BUNDLE"]))
        first = result.get("batches", [{}])[0]
        self.assertEqual(first.get("input_ids"), [[101, 102]])
        self.assertEqual(first.get("token_type_ids"), [[0, 0]])
        self.assertEqual(first.get("attention_mask"), [[1, 1]])


if __name__ == "__main__":
    unittest.main()
