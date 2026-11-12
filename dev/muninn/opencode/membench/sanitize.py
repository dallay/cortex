"""Redaction helpers for stored evidence.

Evidence files are committed to the repository, so they must never contain
credentials, provider tokens, or the embedding vectors that the retrieval
tools return. The rules are explicit and shared by every writer.
"""

from __future__ import annotations

import re
from typing import Any, Mapping

SECRET_KEY_PATTERN = re.compile(
    r"(api[_-]?key|secret|token|password|passwd|credential|authorization|cookie|session[_-]?token)",
    re.IGNORECASE,
)
EMBEDDING_KEY_PATTERN = re.compile(r"^(embedding|embeddings|vector|vectors)$", re.IGNORECASE)
BEARER_PATTERN = re.compile(r"(?i)\b(bearer|basic)\s+[A-Za-z0-9._\-]{8,}")
KEY_LIKE_PATTERN = re.compile(r"\b(sk|pk|ghp|gho|glpat|xox[baprs])-[A-Za-z0-9._\-]{8,}")

REDACTED = "[REDACTED]"
MAX_STRING_LENGTH = 2000


def redact_text(value: str, max_length: int = MAX_STRING_LENGTH) -> str:
    """Remove token-shaped substrings and cap the stored length."""

    cleaned = BEARER_PATTERN.sub(REDACTED, value)
    cleaned = KEY_LIKE_PATTERN.sub(REDACTED, cleaned)
    if len(cleaned) > max_length:
        cleaned = cleaned[:max_length] + f"...[truncated {len(cleaned) - max_length} chars]"
    return cleaned


def sanitize(value: Any, max_length: int = MAX_STRING_LENGTH) -> Any:
    """Return a redacted, JSON-safe copy of ``value``."""

    if isinstance(value, Mapping):
        result: dict[str, Any] = {}
        for key, item in value.items():
            name = str(key)
            if SECRET_KEY_PATTERN.search(name):
                result[name] = REDACTED
                continue
            if EMBEDDING_KEY_PATTERN.match(name):
                result[name] = "[EMBEDDING OMITTED]"
                continue
            result[name] = sanitize(item, max_length)
        return result
    if isinstance(value, (list, tuple)):
        return [sanitize(item, max_length) for item in value]
    if isinstance(value, str):
        return redact_text(value, max_length)
    return value
