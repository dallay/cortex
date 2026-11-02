"""Tests for the pinned module profile.

Run from the repository root with PYTHONPATH=dev/muninn. Set
MUNINN_MINILM_BUNDLE to the pinned bundle to check its actual modules.json.
"""

import hashlib
import io
import json
import os
import sys
from pathlib import Path
import unittest
from unittest.mock import Mock, patch
from types import SimpleNamespace
import tempfile

import minilm_cpu_oracle


class ConstructionTests(unittest.TestCase):
    bundle_id = "ee963db150ae720729aebee53794830f1d0e60960c9253e575a049b799758ac1"

    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name).resolve()
        self.bundle = self.root / self.bundle_id
        self.bundle.mkdir()
        self.output = json.dumps({
            "event": "bundle_verified", "bundle_id": self.bundle_id, "files": 11,
            "artifact_bytes": 91578415, "revision": "1110a243fdf4706b3f48f1d95db1a4f5529b4d41",
            "inference_executed": False})

    def test_preflight_rejects_status_output_and_paths_before_constructor(self):
        missing = self.root / "missing" / self.bundle_id
        wrong = self.root / "wrong"
        wrong.mkdir()
        file_parent = self.root / "file-parent"
        file_parent.mkdir()
        file_bundle = file_parent / self.bundle_id
        file_bundle.touch()
        link_parent = self.root / "linked-parent"
        link_parent.symlink_to(self.bundle.parent, target_is_directory=True)
        link_root = self.root / "link-root"
        link_root.mkdir()
        link_bundle = link_root / self.bundle_id
        link_bundle.symlink_to(self.bundle, target_is_directory=True)
        cases = [
            (str(self.bundle), 1, self.output),
            (str(self.bundle), 0, "not JSON"),
            (self.bundle_id, 0, self.output),
            (str(wrong), 0, self.output),
            (str(missing), 0, self.output),
            (str(file_bundle), 0, self.output),
            (str(link_parent / self.bundle_id), 0, self.output),
            (str(link_bundle), 0, self.output),
            (str(self.bundle / ".." / self.bundle_id), 0, self.output),
        ]
        with patch("sentence_transformers.SentenceTransformer") as constructor:
            for path, status, output in cases:
                with self.subTest(path=path, status=status, output=output):
                    with self.assertRaises(ValueError):
                        minilm_cpu_oracle.construct_cpu_reference(path, status, output)
                    constructor.assert_not_called()

    def test_constructor_and_configuration_errors_propagate(self):
        failure = RuntimeError("constructor failed")
        with patch("sentence_transformers.SentenceTransformer", side_effect=failure) as constructor:
            with self.assertRaises(RuntimeError) as raised:
                minilm_cpu_oracle.construct_cpu_reference(str(self.bundle), 0, self.output)
            self.assertIs(raised.exception, failure)
            constructor.assert_called_once()
        model = Mock()
        model.parameters.return_value = []
        with patch("sentence_transformers.SentenceTransformer", return_value=model) as constructor:
            with self.assertRaises(ValueError):
                minilm_cpu_oracle.construct_cpu_reference(str(self.bundle), 0, self.output)
            constructor.assert_called_once()
            model.encode.assert_not_called()
            model.forward.assert_not_called()

    def test_patched_configuration_error_propagates(self):
        import torch
        model = Mock()
        failure = RuntimeError("configuration failed")
        with patch("sentence_transformers.SentenceTransformer", return_value=model) as constructor:
            with patch.object(minilm_cpu_oracle, "configure_cpu_reference", side_effect=failure) as configure:
                with self.assertRaises(RuntimeError) as raised:
                    minilm_cpu_oracle.construct_cpu_reference(str(self.bundle), 0, self.output)
        self.assertIs(raised.exception, failure)
        constructor.assert_called_once()
        configure.assert_called_once_with(model, torch.float32)
        model.encode.assert_not_called()
        model.forward.assert_not_called()

    def test_constructs_exact_local_path_and_configures_reference(self):
        import torch
        model = Mock(training=True, max_seq_length=128)
        model.parameters.return_value = [SimpleNamespace(
            device=SimpleNamespace(type="cpu"), dtype=torch.float32)]
        model.buffers.return_value = []
        model.modules.return_value = [model]
        model.eval.side_effect = lambda: setattr(model, "training", False)
        with patch("sentence_transformers.SentenceTransformer", return_value=model) as constructor:
            result = minilm_cpu_oracle.construct_cpu_reference(str(self.bundle), 0, self.output)
        constructor.assert_called_once_with(str(self.bundle), **local_loader_options(torch.float32))
        self.assertIs(result, model)
        model.eval.assert_called_once_with()
        self.assertIs(model.training, False)
        self.assertEqual(model.max_seq_length, 256)
        model.encode.assert_not_called()
        model.forward.assert_not_called()

from minilm_cpu_oracle import validate_modules, load_pinned_lock, verify_stream, verify_bundle_event, validate_pooling, local_loader_options, configure_cpu_reference


class ReferenceConfigurationTests(unittest.TestCase):
    def test_configures_eval_and_256_without_casting_wrong_weights(self):
        from types import SimpleNamespace
        import torch
        dtype = torch.float32

        class Model:
            max_seq_length = 128
            training = True

            def parameters(self):
                return iter([SimpleNamespace(device=SimpleNamespace(type="cpu"), dtype=dtype)])

            def buffers(self):
                return iter([])

            def modules(self):
                return iter([self])

            def eval(self):
                self.training = False
                return self

        half_model = Model()
        half_model.parameters = lambda: iter([SimpleNamespace(device=SimpleNamespace(type="cpu"), dtype=torch.float16)])
        with self.assertRaises(ValueError):
            configure_cpu_reference(half_model, torch.float16)
        model = Model()
        configure_cpu_reference(model, dtype)
        self.assertEqual(model.max_seq_length, 256)
        self.assertIs(model.training, False)
        for parameters in ([], [SimpleNamespace(device=SimpleNamespace(type="cuda"), dtype=dtype)],
                           [SimpleNamespace(device=SimpleNamespace(type="cpu"), dtype="wrong")]):
            model.parameters = lambda values=parameters: iter(values)
            with self.assertRaises(ValueError):
                configure_cpu_reference(model, dtype)
        model = Model()
        integer = SimpleNamespace(device=SimpleNamespace(type="cpu"), dtype=torch.int64,
                                  is_floating_point=lambda: False)
        model.buffers = lambda: iter([integer])
        configure_cpu_reference(model, dtype)
        for buffer in (SimpleNamespace(device=SimpleNamespace(type="cuda"), is_floating_point=lambda: False),
                       SimpleNamespace(device=SimpleNamespace(type="cpu"), dtype=torch.float16,
                                       is_floating_point=lambda: True)):
            model.buffers = lambda value=buffer: iter([value])
            with self.assertRaises(ValueError):
                configure_cpu_reference(model, dtype)
        model = Model()
        model.modules = lambda: iter([model, SimpleNamespace(training=True)])
        with self.assertRaises(ValueError):
            configure_cpu_reference(model, dtype)


class LoaderOptionsTests(unittest.TestCase):
    def test_local_cpu_loading_requires_safe_explicit_flags(self):
        dtype = object()
        options = local_loader_options(dtype)
        self.assertEqual(options["device"], "cpu")
        self.assertEqual(options["backend"], "torch")
        self.assertIs(options["local_files_only"], True)
        self.assertIs(options["trust_remote_code"], False)
        self.assertIs(options["token"], False)
        self.assertIs(options["model_kwargs"]["dtype"], dtype)
        for name in ("use_safetensors", "weights_only", "disable_mmap"):
            self.assertIs(options["model_kwargs"][name], True)
        self.assertNotIn("attn_implementation", options["model_kwargs"])
        for name, value in (("local_files_only", True), ("trust_remote_code", False), ("token", False)):
            self.assertIs(options["model_kwargs"][name], value)
        for key in ("processor_kwargs", "config_kwargs"):
            self.assertEqual(options[key], {"local_files_only": True, "trust_remote_code": False, "token": False})


class PoolingTests(unittest.TestCase):
    def test_requires_exact_mean_pooling_profile(self):
        profile = {"word_embedding_dimension": 384, "pooling_mode_cls_token": False,
                   "pooling_mode_mean_tokens": True, "pooling_mode_max_tokens": False,
                   "pooling_mode_mean_sqrt_len_tokens": False}
        validate_pooling(profile)
        for changed in ({**profile, "word_embedding_dimension": 768},
                        {**profile, "pooling_mode_mean_tokens": 1},
                        {**profile, "pooling_mode_max_tokens": True},
                        {**profile, "unexpected": False}):
            with self.subTest(profile=changed), self.assertRaises(ValueError):
                validate_pooling(changed)


class VerifierEventTests(unittest.TestCase):
    def test_only_accepts_successful_final_pinned_event(self):
        bundle = "ee963db150ae720729aebee53794830f1d0e60960c9253e575a049b799758ac1"
        event = {"event": "bundle_verified", "bundle_id": bundle, "files": 11,
                 "artifact_bytes": 91578415, "revision": "1110a243fdf4706b3f48f1d95db1a4f5529b4d41",
                 "inference_executed": False}
        output = json.dumps({"event": "cache_artifact_verified"}) + "\n" + json.dumps(event) + "\n"
        verify_bundle_event(0, output)
        duplicate = json.dumps(event).replace('"inference_executed": false', '"inference_executed": true, "inference_executed": false')
        cases = [(False, output), (0.0, output), (0, duplicate), (0, " " * 65537),
                 (1, output), (0, ""), (0, json.dumps(event) + "\n{}\n"),
                 (0, json.dumps({**event, "bundle_id": "0" * 64})),
                 (0, json.dumps({**event, "inference_executed": 0})),
                 (0, json.dumps({**event, "files": 10}))]
        for status, text in cases:
            with self.subTest(status=status, text=text), self.assertRaises(ValueError):
                verify_bundle_event(status, text)


class StreamTests(unittest.TestCase):
    def test_rejects_stream_exceeding_requested_read(self):
        class OverRead:
            calls = 0

            def read(self, count):
                self.calls += 1
                return b"x" * (count + 1)

        stream = OverRead()
        with self.assertRaises(ValueError):
            verify_stream(stream, 2 * 1024 * 1024, "unused")
        self.assertEqual(stream.calls, 1)

    def test_incremental_verification_and_size_boundaries(self):
        class ShortReads(io.BytesIO):
            def read(self, count=-1):
                return super().read(min(count, 2))

        data = b"abcdef"
        digest = hashlib.sha256(data).hexdigest()
        verify_stream(ShortReads(data), len(data), digest)
        for changed in (data[:-1], data + b"x", b"abcdeg"):
            with self.subTest(data=changed), self.assertRaises(ValueError):
                verify_stream(ShortReads(changed), len(data), digest)


class EncodingTests(unittest.TestCase):
    @staticmethod
    def context_state(torch):
        return (torch.is_grad_enabled(), torch.is_inference_mode_enabled(),
                torch.is_autocast_enabled("cpu"))

    def test_encode_preserves_arguments_result_and_outer_contexts(self):
        import torch
        before = self.context_state(torch)
        sentences = ["reference sentence", "another sentence"]
        result = object()
        model = Mock()

        def encode(*args, **kwargs):
            self.assertEqual(self.context_state(torch), (False, True, False))
            self.assertIs(args[0], sentences)
            self.assertEqual(args, (sentences,))
            self.assertEqual(kwargs, {"convert_to_numpy": True})
            return result

        model.encode.side_effect = encode
        with torch.enable_grad(), torch.autocast("cpu", dtype=torch.bfloat16):
            outer = self.context_state(torch)
            self.assertEqual(outer, (True, before[1], True))
            actual = minilm_cpu_oracle.encode_cpu_reference(model, sentences)
            self.assertIs(actual, result)
            self.assertEqual(self.context_state(torch), outer)
        self.assertEqual(self.context_state(torch), before)
        model.encode.assert_called_once_with(sentences, convert_to_numpy=True)
        model.forward.assert_not_called()

    def test_encode_error_propagates_and_restores_outer_contexts(self):
        import torch
        before = self.context_state(torch)
        sentences = ["reference sentence"]
        failure = RuntimeError("encode failed")
        model = Mock()

        def encode(*args, **kwargs):
            self.assertEqual(self.context_state(torch), (False, True, False))
            raise failure

        model.encode.side_effect = encode
        with torch.enable_grad(), torch.autocast("cpu", dtype=torch.bfloat16):
            outer = self.context_state(torch)
            self.assertEqual(outer, (True, before[1], True))
            with self.assertRaises(RuntimeError) as raised:
                minilm_cpu_oracle.encode_cpu_reference(model, sentences)
            self.assertIs(raised.exception, failure)
            self.assertEqual(self.context_state(torch), outer)
        self.assertEqual(self.context_state(torch), before)
        model.encode.assert_called_once_with(sentences, convert_to_numpy=True)
        model.forward.assert_not_called()


class LockTests(unittest.TestCase):
    def test_lock_authentication_rejects_one_byte_change(self):
        root = Path(__file__).resolve().parents[3]
        data = (root / "docs/muninn/evidence/artifact-lock.json").read_bytes()
        lock = load_pinned_lock(data)
        self.assertEqual(lock["schema_version"], 1)
        for changed in (data + b" ", bytes([data[0] ^ 1]) + data[1:]):
            with self.subTest(size=len(changed)), self.assertRaises(ValueError):
                load_pinned_lock(changed)


class ModuleProfileTests(unittest.TestCase):
    def test_rejects_module_type_or_path_changes(self):
        modules = [
            {"idx": 0, "name": "0", "path": "", "type": "sentence_transformers.models.Transformer"},
            {"idx": 1, "name": "1", "path": "1_Pooling", "type": "sentence_transformers.models.Pooling"},
            {"idx": 2, "name": "2", "path": "2_Normalize", "type": "sentence_transformers.models.Normalize"},
        ]
        validate_modules(modules)
        for key, value in (("type", "evil.Model"), ("path", "../other")):
            changed = [item.copy() for item in modules]
            changed[1][key] = value
            with self.subTest(key=key), self.assertRaises(ValueError):
                validate_modules(changed)

    def test_rejects_wrong_structure_and_boolean_index(self):
        good = [
            {"idx": 0, "name": "0", "path": "", "type": "sentence_transformers.models.Transformer"},
            {"idx": 1, "name": "1", "path": "1_Pooling", "type": "sentence_transformers.models.Pooling"},
            {"idx": 2, "name": "2", "path": "2_Normalize", "type": "sentence_transformers.models.Normalize"},
        ]
        for value in (good[:-1], good[::-1], [{**good[0], "idx": False}, *good[1:]],
                      [{**good[0], "extra": 1}, *good[1:]], [{"idx": 0}, *good[1:]]):
            with self.subTest(value=value), self.assertRaises(ValueError):
                validate_modules(value)

    @unittest.skipUnless(os.environ.get("MUNINN_MINILM_BUNDLE"), "Requires the explicit pinned local bundle")
    def test_pinned_bundle_modules_match_locked_bytes(self):
        root = Path(__file__).resolve().parents[3]
        lock = load_pinned_lock((root / "docs/muninn/evidence/artifact-lock.json").read_bytes())
        model = next(m for m in lock["models"] if m["id"] == "sentence-transformers/all-MiniLM-L6-v2")
        self.assertEqual(model["revision"], "1110a243fdf4706b3f48f1d95db1a4f5529b4d41")
        artifact = next(f for f in model["files"] if f["path"] == "modules.json")
        data = (Path(os.environ["MUNINN_MINILM_BUNDLE"]) / "modules.json").read_bytes()
        self.assertEqual(len(data), artifact["size"])
        self.assertEqual(hashlib.sha256(data).hexdigest(), artifact["sha256"])
        validate_modules(json.loads(data))


if __name__ == "__main__":
    unittest.main()
