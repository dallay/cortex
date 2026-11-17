"""Pruebas sin servicios del comparador GM-01."""

import copy
import unittest


def capture(prefix="r"):
    metadata = {"id": "conservar", "timestamp": "conservar", "nested": {"tags": ["ñ", 1]}}
    sessions = []
    for index, contents in enumerate((["Hola, Muñinn 🧠\nsegunda línea", "duplicado", "duplicado"], ["otra sesión"])):
        cid = f"{prefix}-c{index}"
        sessions.append({"id": cid, "session_id": f"sesion-{index}", "messages": [
            {"id": f"{prefix}-m{index}-{n}", "conversation_id": cid, "role": "user" if index == 0 else "assistant",
             "content": text, "metadata": copy.deepcopy(metadata), "timestamp": f"2026-11-01T12:00:0{n}+00:00"}
            for n, text in enumerate(contents)
        ]})
    return {"conversations": sessions}


class DifferentialTests(unittest.TestCase):
    def test_accepts_distinct_generated_ids_without_losing_metadata(self):
        from gm01_differential import compare
        result = compare(capture(), capture("rust"))
        self.assertEqual(result["comparison"], "pass")
        self.assertEqual(result["h1_acceptance"], "not_established")

    def test_detects_semantic_changes(self):
        from gm01_differential import compare
        for field, value in (("content", "cambio"), ("role", "system"),
                             ("metadata", {"nested": {"tags": ["n", 1]}})):
            with self.subTest(field=field):
                changed = capture("rust")
                changed["conversations"][0]["messages"][0][field] = value
                self.assertEqual(compare(capture(), changed)["comparison"], "fail")

    def test_detects_order_and_duplicate_loss(self):
        from gm01_differential import compare
        for operation in ("reverse", "pop"):
            changed = capture("rust")
            getattr(changed["conversations"][0]["messages"], operation)()
            self.assertEqual(compare(capture(), changed)["comparison"], "fail")

    def test_rejects_broken_identity_and_time(self):
        from gm01_differential import compare
        for field, value in (("id", "rust-m0-1"), ("conversation_id", "otro"),
                             ("timestamp", "invalido"), ("timestamp", "2026-11-01T12:00:00")):
            with self.subTest(field=field, value=value):
                changed = capture("rust")
                changed["conversations"][0]["messages"][0][field] = value
                self.assertEqual(compare(capture(), changed)["comparison"], "fail")

    def test_rejects_incomplete_or_unrecognized_capture(self):
        from gm01_differential import compare
        for changed in ({}, {"conversations": []}, {"conversations": capture()["conversations"], "unexpected": True}):
            self.assertEqual(compare(capture(), changed)["comparison"], "fail")

    def test_rejects_ambiguous_json_and_nonfinite_numbers(self):
        from gm01_differential import load_capture, CaptureError, compare
        for raw in ('{"conversations": [], "conversations": []}', '{"metadata": {"x": NaN}}'):
            with self.subTest(raw=raw), self.assertRaises(CaptureError):
                load_capture(raw)
        bad = capture()
        bad["conversations"][0]["messages"][0]["metadata"]["number"] = float("inf")
        self.assertEqual(compare(bad, bad)["comparison"], "fail")

    def test_validates_reference_too(self):
        from gm01_differential import compare
        bad = capture()
        bad["conversations"][1]["id"] = bad["conversations"][0]["id"]
        self.assertEqual(compare(bad, capture("rust"))["comparison"], "fail")


if __name__ == "__main__":
    unittest.main()
