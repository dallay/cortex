"""Pinned tokenizer-only reference; no sentence model or remote code execution."""

from pathlib import Path
import hashlib
import importlib.metadata
import json
import os
import stat
import sys


def _checked_files(bundle: Path, model: dict) -> dict:
    if not bundle.is_absolute() or any(part in (".", "..") for part in bundle.parts):
        raise ValueError("Expected a physical absolute bundle path")
    directory = os.open("/", os.O_RDONLY | os.O_DIRECTORY)
    try:
        for component in bundle.parts[1:]:
            child = os.open(component, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW, dir_fd=directory)
            os.close(directory)
            directory = child
        metadata = os.fstat(directory)
        if metadata.st_uid != os.geteuid() or stat.S_IMODE(metadata.st_mode) != 0o500:
            raise ValueError("Unsafe reference bundle")
        expected_names = {item["path"].split("/", 1)[0] for item in model["files"]} | {"_muninn_artifacts.json"}
        if set(os.listdir(directory)) != expected_names:
            raise ValueError("Unexpected bundle inventory")
        records = {}
        selected = {"config.json", "tokenizer.json", "tokenizer_config.json", "special_tokens_map.json", "sentence_bert_config.json", "vocab.txt"}
        for item in model["files"]:
            if item["path"] not in selected:
                continue
            descriptor = os.open(item["path"], os.O_RDONLY | os.O_NOFOLLOW, dir_fd=directory)
            try:
                metadata = os.fstat(descriptor)
                if not stat.S_ISREG(metadata.st_mode) or metadata.st_uid != os.geteuid() or stat.S_IMODE(metadata.st_mode) != 0o400:
                    raise ValueError("Unsafe reference artifact")
                data = os.read(descriptor, item["size"] + 1)
                digest = hashlib.sha256(data).hexdigest()
                if len(data) != item["size"] or digest != item["sha256"]:
                    raise ValueError("Reference artifact mismatch")
                records[item["path"]] = {"bytes":len(data), "sha256":digest}
            finally:
                os.close(descriptor)
        if set(records) != selected:
            raise ValueError("Incomplete tokenizer artifacts")
        return records
    finally:
        os.close(directory)


def _cases() -> list:
    return [
        ("empty", [""]),
        ("basic", ["Hello world!"]),
        ("unicode", ["Café naïve résumé", "Cafe\u0301 and re\u0301sume\u0301", "中文，世界。", "👩‍💻 🚀", "İSTANBUL Straße"]),
        ("controls_and_whitespace", [" \t\n", "a\u0000b\t\nhello\u001c world", "one\u00a0two\u2003three"]),
        ("literal_specials", ["[CLS] hello [SEP]", "[PAD] [UNK] [MASK]"]),
        ("content_126", ["hello " * 126]),
        ("content_254", ["hello " * 254]),
        ("content_255", ["hello " * 255]),
        ("content_500", ["hello " * 500]),
        ("mixed", ["", "Hello world!", "hello " * 300]),
    ]


def generate(bundle: Path) -> dict:
    project = Path(__file__).resolve().parents[2]
    lock_path = project / "docs/muninn/evidence/artifact-lock.json"
    lock_bytes = lock_path.read_bytes()
    lock = json.loads(lock_bytes)
    model = next(item for item in lock["models"] if item["id"] == "sentence-transformers/all-MiniLM-L6-v2")
    packages = {name:importlib.metadata.version(name) for name in ("transformers", "tokenizers", "huggingface-hub", "safetensors")}
    if any(version != lock["framework_packages"][name] for name, version in packages.items()):
        raise ValueError("Reference package version mismatch")
    requirements = (project / "dev/muninn/requirements-minilm-tokenizer.txt").read_bytes()
    for line in requirements.decode().splitlines():
        if not line or line.startswith("#"):
            continue
        name, version = line.split(" --hash=sha256:", 1)[0].split("==", 1)
        if importlib.metadata.version(name) != version:
            raise ValueError("Reference dependency lock mismatch")
    artifacts = _checked_files(bundle, model)
    if any(os.environ.get(name) != "1" for name in ("HF_HUB_OFFLINE", "TRANSFORMERS_OFFLINE")) or os.environ.get("USE_TORCH") != "0":
        raise ValueError("Offline tokenizer-only environment is required")
    from transformers import AutoTokenizer

    tokenizer = AutoTokenizer.from_pretrained(bundle, local_files_only=True, trust_remote_code=False)
    if not tokenizer.is_fast:
        raise ValueError("Expected the pinned fast tokenizer")
    tokenizer.padding_side = "right"
    tokenizer.truncation_side = "right"
    tokenizer.model_max_length = 256
    batches = []
    for name, texts in _cases():
        encoded = tokenizer(texts, add_special_tokens=True, padding=True, truncation="longest_first", max_length=256,
            return_attention_mask=True, return_token_type_ids=True)
        batches.append({"id":name, "texts":texts, **dict(encoded)})
    if _checked_files(bundle, model) != artifacts:
        raise ValueError("Reference artifacts changed during generation")
    return {"schema_version":1, "scope":"Pinned fast-tokenizer differential only; no sentence model or inference",
        "reference_commit":lock["reference_commit"], "model_id":model["id"], "revision":model["revision"],
        "artifact_lock_sha256":hashlib.sha256(lock_bytes).hexdigest(), "artifacts":artifacts,
        "reference_requirements_sha256":hashlib.sha256(requirements).hexdigest(),
        "packages":packages, "python_version":sys.version.split()[0], "tokenizer_class":type(tokenizer).__name__,
        "fast_backend":tokenizer.is_fast,
        "effective_settings":{"add_special_tokens":True,"padding":"longest","padding_side":"right",
            "truncation":"longest_first","truncation_side":"right","stride":0,"max_length":256},
        "generator_sha256":hashlib.sha256(Path(__file__).read_bytes()).hexdigest(),
        "sentence_transformers_runtime_executed":False, "model_forward_executed":False,
        "max_length_including_specials":256, "batches":batches}


if __name__ == "__main__":
    if len(sys.argv) != 2:
        raise SystemExit("Expected one operator-supplied pinned bundle directory")
    print(json.dumps(generate(Path(sys.argv[1])), ensure_ascii=False, indent=2))
