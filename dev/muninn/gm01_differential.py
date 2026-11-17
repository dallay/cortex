"""Comparación offline GM-01; no ejecuta inferencia ni accede a servicios."""

import argparse
from datetime import datetime
import json
from pathlib import Path


class CaptureError(ValueError):
    """La captura no satisface el contrato de comparación."""


def _fields(value, expected):
    if not isinstance(value, dict) or set(value) != set(expected):
        raise CaptureError("campos de captura incorrectos")


def _identity(value):
    if not isinstance(value, str) or not value.strip():
        raise CaptureError("identidad vacía o inválida")
    return value


def _canonical(document):
    _fields(document, ("conversations",))
    conversations = document["conversations"]
    if not isinstance(conversations, list) or len(conversations) != 2:
        raise CaptureError("GM-01 requiere exactamente dos sesiones")
    ids, sessions, normalized = set(), set(), []
    for conversation in conversations:
        _fields(conversation, ("id", "session_id", "messages"))
        cid = _identity(conversation["id"])
        session = _identity(conversation["session_id"])
        if cid in ids or session in sessions:
            raise CaptureError("identidad de conversación o sesión repetida")
        ids.add(cid)
        sessions.add(session)
        messages = conversation["messages"]
        if not isinstance(messages, list) or not messages:
            raise CaptureError("sesión sin mensajes")
        payloads, previous = [], None
        for message in messages:
            _fields(message, ("id", "conversation_id", "role", "content", "metadata", "timestamp"))
            mid = _identity(message["id"])
            if mid in ids or message["conversation_id"] != cid:
                raise CaptureError("identidad repetida o referencia de conversación rota")
            ids.add(mid)
            if message["role"] not in ("user", "assistant", "system", "tool"):
                raise CaptureError("rol inválido")
            if not isinstance(message["content"], str) or not isinstance(message["metadata"], dict):
                raise CaptureError("contenido o metadata inválidos")
            try:
                stamp = datetime.fromisoformat(message["timestamp"])
            except (TypeError, ValueError) as error:
                raise CaptureError("timestamp inválido") from error
            if stamp.utcoffset() is None or (previous is not None and stamp < previous):
                raise CaptureError("timestamp sin zona u orden temporal incorrecto")
            previous = stamp
            payloads.append({key: message[key] for key in ("role", "content", "metadata")})
        normalized.append({"session_id": session, "messages": payloads})
    # JSON canónico distingue bool/int y conserva listas, Unicode y metadata anidada.
    try:
        return json.dumps(normalized, ensure_ascii=False, sort_keys=True, allow_nan=False)
    except (TypeError, ValueError) as error:
        raise CaptureError("payload no representable como JSON") from error


def compare(reference, candidate):
    """Valida ambas capturas antes de normalizar identidad y tiempo generados."""
    errors = []
    normalized = []
    for label, document in (("referencia", reference), ("candidato", candidate)):
        try:
            normalized.append(_canonical(document))
        except CaptureError as error:
            errors.append(f"{label}: {error}")
    if not errors and normalized[0] != normalized[1]:
        errors.append("contenido, sesiones, roles, metadata u orden diferentes")
    return {"comparison": "fail" if errors else "pass", "errors": errors,
            "h1_acceptance": "not_established", "execution": "offline_capture_comparison"}


def load_capture(raw):
    """Rechaza JSON ambiguo y constantes no finitas en cualquier nivel."""
    def pairs(items):
        result = {}
        for key, value in items:
            if key in result:
                raise CaptureError("clave JSON repetida")
            result[key] = value
        return result

    def constant(_value):
        raise CaptureError("constante numérica no admitida en JSON")

    return json.loads(raw, object_pairs_hook=pairs, parse_constant=constant)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("reference", type=Path)
    parser.add_argument("candidate", type=Path)
    args = parser.parse_args()
    try:
        result = compare(load_capture(args.reference.read_text()), load_capture(args.candidate.read_text()))
    except (OSError, ValueError) as error:
        parser.exit(2, f"No se pueden leer las capturas: {type(error).__name__}\n")
    print(json.dumps(result, ensure_ascii=False, sort_keys=True))
    return 0 if result["comparison"] == "pass" else 1


if __name__ == "__main__":
    raise SystemExit(main())
