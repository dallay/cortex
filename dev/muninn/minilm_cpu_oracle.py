"""Pinned CPU reference helpers; model construction is explicit and local-only."""

import hashlib
import json
from pathlib import Path


def construct_cpu_reference(bundle_path, verification_status, verification_output):
    """Construct without inference from a trusted, stable local bundle.

    The caller must externally verify this bundle immediately before this call
    and keep it stable throughout loading and use. Status/output are not bound
    cryptographically to the pathname and are not safe authorization for hostile
    inputs. Path checks do not prevent concurrent replacement or upstream reopen.
    """
    verify_bundle_event(verification_status, verification_output)
    path = Path(bundle_path)
    if (not path.is_absolute()
            or path.name != "ee963db150ae720729aebee53794830f1d0e60960c9253e575a049b799758ac1"
            or ".." in path.parts
            or any(component.is_symlink() for component in (path, *path.parents))
            or not path.is_dir()):
        raise ValueError("Expected existing absolute pinned bundle without symlinks")

    import torch
    from sentence_transformers import SentenceTransformer

    model = SentenceTransformer(str(bundle_path), **local_loader_options(torch.float32))
    configure_cpu_reference(model, torch.float32)
    return model

def encode_cpu_reference(model, sentences):
    """Encode with upstream defaults under CPU inference without autocast."""
    import torch

    with torch.inference_mode(), torch.autocast(device_type="cpu", enabled=False):
        return model.encode(sentences, convert_to_numpy=True)


_LOCK_SIZE = 11734
_LOCK_SHA256 = "c0a09354337251cd7e35b81e0f505a46bf4086edb6c5506bd34b4df9259a9794"


def load_pinned_lock(data):
    """Authenticate exact lock bytes before parsing their contents."""
    if not isinstance(data, bytes) or len(data) != _LOCK_SIZE or hashlib.sha256(data).hexdigest() != _LOCK_SHA256:
        raise ValueError("Unexpected artifact lock")
    return json.loads(data)


def verify_stream(stream, size, expected_sha256):
    """Hash exactly size bytes with bounded reads and reject trailing data."""
    if type(size) is not int or size < 0:
        raise ValueError("Invalid artifact size")
    digest = hashlib.sha256()
    remaining = size
    while remaining:
        requested = min(remaining, 1024 * 1024)
        chunk = stream.read(requested)
        if not chunk or len(chunk) > requested:
            raise ValueError("Unexpected artifact size")
        digest.update(chunk)
        remaining -= len(chunk)
    if stream.read(1) or digest.hexdigest() != expected_sha256:
        raise ValueError("Unexpected artifact bytes")


def _unique_object(pairs):
    result = {}
    for key, value in pairs:
        if key in result:
            raise ValueError("Duplicate JSON key")
        result[key] = value
    return result


def verify_bundle_event(status, output):
    """Require the pinned verifier's final successful bundle event."""
    if type(status) is not int or status != 0 or not isinstance(output, str) or len(output) > 65536:
        raise ValueError("Bundle verification failed")
    try:
        lines = output.splitlines()
        event = json.loads(lines[-1], object_pairs_hook=_unique_object)
    except (IndexError, ValueError):
        raise ValueError("Missing bundle verification event") from None
    if not isinstance(event, dict) or event != {
        "event": "bundle_verified",
        "bundle_id": "ee963db150ae720729aebee53794830f1d0e60960c9253e575a049b799758ac1",
        "files": 11,
        "artifact_bytes": 91578415,
        "revision": "1110a243fdf4706b3f48f1d95db1a4f5529b4d41",
        "inference_executed": False,
    } or type(event.get("files")) is not int or type(event.get("artifact_bytes")) is not int or event.get("inference_executed") is not False:
        raise ValueError("Unexpected bundle verification event")


def configure_cpu_reference(model, dtype):
    """Reject wrong parameter placement/dtype, set evaluation and length 256.

    This does not establish tokenizer behavior, no-grad, or numerical parity.
    Integer buffers are permitted; floating buffers must use the requested dtype.
    """
    import torch

    if dtype is not torch.float32:
        raise ValueError("Expected F32 reference dtype")
    count = 0
    for parameter in model.parameters():
        count += 1
        if parameter.device.type != "cpu" or parameter.dtype != dtype:
            raise ValueError("Expected CPU/F32 parameters")
    if not count:
        raise ValueError("Missing model parameters")
    for buffer in model.buffers():
        if buffer.device.type != "cpu" or (buffer.is_floating_point() and buffer.dtype != dtype):
            raise ValueError("Unexpected model buffer")
    model.max_seq_length = 256
    model.eval()
    if type(model.max_seq_length) is not int or model.max_seq_length != 256 or any(
        module.training is not False for module in model.modules()
    ):
        raise ValueError("Reference configuration did not take effect")


def local_loader_options(dtype):
    """Explicit local CPU loader policy; does not construct or execute a model."""
    local = {"local_files_only": True, "trust_remote_code": False, "token": False}
    return {"device": "cpu", "backend": "torch", **local,
            "model_kwargs": {"dtype": dtype, "use_safetensors": True,
                             "weights_only": True, "disable_mmap": True, **local},
            "processor_kwargs": dict(local), "config_kwargs": dict(local)}


def validate_pooling(profile):
    """Require the exact pinned 384-dimensional masked mean profile."""
    expected = {"word_embedding_dimension": 384, "pooling_mode_cls_token": False,
                "pooling_mode_mean_tokens": True, "pooling_mode_max_tokens": False,
                "pooling_mode_mean_sqrt_len_tokens": False}
    if not isinstance(profile, dict) or profile != expected or any(
        type(profile[key]) is not type(value) for key, value in expected.items()
    ):
        raise ValueError("Unexpected pooling profile")


_EXPECTED_MODULES = (
    (0, "0", "", "sentence_transformers.models.Transformer"),
    (1, "1", "1_Pooling", "sentence_transformers.models.Pooling"),
    (2, "2", "2_Normalize", "sentence_transformers.models.Normalize"),
)


def validate_modules(modules):
    """Reject any deviation from the pinned module sequence and paths."""
    if not isinstance(modules, list) or len(modules) != len(_EXPECTED_MODULES):
        raise ValueError("Unexpected module profile")
    for module, expected in zip(modules, _EXPECTED_MODULES):
        if not isinstance(module, dict) or set(module) != {"idx", "name", "path", "type"}:
            raise ValueError("Unexpected module profile")
        if type(module["idx"]) is not int or tuple(module[k] for k in ("idx", "name", "path", "type")) != expected:
            raise ValueError("Unexpected module profile")
