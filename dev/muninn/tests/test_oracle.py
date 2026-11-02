"""Verifica la normalización sin perder datos del usuario."""

import importlib.util
from pathlib import Path
import unittest


class OracleTests(unittest.TestCase):
    def test_session_check_rejects_wrong_messages_even_when_counts_match(self):
        script = Path(__file__).resolve().parents[1] / "oracle.py"
        spec = importlib.util.spec_from_file_location("oracle", script)
        module = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(module)
        first = {"id": "conversation-a", "messages": [
            {"id": "message-a", "role": "user", "content": "A", "metadata": {}}
        ]}
        second = {"id": "conversation-b", "messages": [
            {"id": "message-b", "role": "assistant", "content": "B", "metadata": {}}
        ]}
        module.verify_sessions([first, second], [first, second])
        with self.assertRaisesRegex(ValueError, "sesion"):
            module.verify_sessions([{"id": "conversation-a", "messages": second["messages"]}, second],
                                   [first, second])
        with self.assertRaisesRegex(ValueError, "sesion"):
            module.verify_sessions([first, {"id": "conversation-b", "messages": first["messages"]}],
                                   [first, second])

    def test_failed_deletion_is_evidence_not_a_passing_contract(self):
        script = Path(__file__).resolve().parents[1] / "oracle.py"
        spec = importlib.util.spec_from_file_location("oracle", script)
        module = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(module)
        failure = module.deletion_observation(
            None, "Neo.ClientError.Schema.ConstraintValidationFailed", ["a", "", "b"], ["a", "b"])
        self.assertEqual(failure["normative_verdict"], "FAIL")
        self.assertIsNone(failure["returned"])
        success = module.deletion_observation(True, None, ["a", "b"], ["a", "b"])
        self.assertEqual(success["normative_verdict"], "PASS")
        corrupt = module.deletion_observation(True, None, ["a", "", "b"], ["a", "b"])
        self.assertEqual(corrupt["normative_verdict"], "FAIL")

    def test_message_normalization_preserves_nested_metadata(self):
        script = Path(__file__).resolve().parents[1] / "oracle.py"
        self.assertTrue(script.is_file(), "Falta el capturador del oráculo")
        spec = importlib.util.spec_from_file_location("oracle", script)
        module = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(module)
        message = {
            "id": "volatile", "timestamp": "volatile", "role": "user",
            "created_at": "volatile", "updated_at": None, "conversation_id": "volatile",
            "content": "Muñinn 🧠\n", "metadata": {"id": "keep", "timestamp": "keep"},
        }
        result = module.normalize_message(message)
        self.assertEqual(result, {
            "role": "user", "content": "Muñinn 🧠\n",
            "metadata": {"id": "keep", "timestamp": "keep"},
        })
        self.assertIn("id", message)


if __name__ == "__main__":
    unittest.main()
