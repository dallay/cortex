"""Captura mínima del oráculo contra el Neo4j efímero de compose.yaml."""

import argparse
import asyncio
from importlib.metadata import version
import json
from pathlib import Path
import platform
import sys
from tempfile import TemporaryDirectory


def normalize_message(message):
    """Elimina solo identidad y tiempo de primer nivel, nunca metadatos."""
    return {key: value for key, value in message.items() if key not in {
        "id", "conversation_id", "timestamp", "created_at", "updated_at"
    }}


def deletion_observation(returned, error_code, remaining, expected):
    """Distingue observar al oráculo de satisfacer el contrato normativo."""
    return {
        "returned": returned, "error_code": error_code,
        "normative_verdict": "PASS" if returned is True and error_code is None and remaining == expected else "FAIL",
    }


def verify_sessions(actual, expected):
    """Comprueba identidades y payloads exactos en ambas sesiones."""
    if len(actual) != 2 or actual[0]["id"] == actual[1]["id"]:
        raise ValueError("Identidades de sesion incorrectas")
    for conversation, wanted in zip(actual, expected, strict=True):
        fields = ("id", "role", "content", "metadata")
        messages = [{k: m[k] for k in fields} for m in conversation["messages"]]
        wanted_messages = [{k: m[k] for k in fields} for m in wanted["messages"]]
        if str(conversation["id"]) != str(wanted["id"]) or messages != wanted_messages:
            raise ValueError("Contenido o identidad de sesion incorrectos")


class UnusedEmbedder:
    """Esta captura no permite llamadas a proveedores ni genera vectores."""

    dimensions = 8

    async def embed(self, text):
        raise RuntimeError("La captura sin embeddings no debe invocar embed")

    async def embed_batch(self, texts):
        raise RuntimeError("La captura sin embeddings no debe invocar embed_batch")


async def capture(password):
    from neo4j import AsyncGraphDatabase
    from neo4j.exceptions import ConstraintError
    from neo4j_agent_memory import MemoryClient
    from neo4j_agent_memory.config.settings import MemorySettings

    uri = "bolt://neo4j:7687"
    async with AsyncGraphDatabase.driver(uri, auth=("neo4j", password)) as driver:
        records, _, _ = await driver.execute_query(
            "MATCH (n) RETURN count(n) AS count", database_="neo4j")
        if records[0]["count"]:
            raise ValueError("El oráculo requiere una base efímera vacía; no se borran datos")
        settings = MemorySettings(
            backend="bolt", neo4j={"uri": uri, "username": "neo4j", "password": password},
            extraction={"extractor_type": "none"}, embedding={"dimensions": 8},
        )
        async with MemoryClient(settings, embedder=UnusedEmbedder()) as client:
            session = "muninn-oracle-gm01"
            contents = ["Hola, Muñinn 🧠\nsegunda línea", "", "x" * 10001]
            metadata = {"id": "conservar", "timestamp": "conservar", "nested": {"tags": ["ñ", 1]}}
            messages = []
            for content in contents:
                messages.append(await client.short_term.add_message(
                    session, "user", content, metadata=metadata,
                    extract_entities=False, extract_relations=False,
                    generate_embedding=False, extraction_mode="skip",
                ))
            conversation = await client.short_term.get_conversation(session)
            if [m.content for m in conversation.messages] != contents:
                raise ValueError("El oráculo no preservó contenido u orden")
            if any(m.metadata != metadata for m in conversation.messages):
                raise ValueError("El oráculo no preservó metadatos")
            repeat = await client.short_term.get_conversation(session)
            if repeat.id != conversation.id:
                raise ValueError("El oráculo cambió la identidad de conversación")
            other_message = await client.short_term.add_message(
                "muninn-oracle-other", "assistant", "otra sesión",
                extract_entities=False, extract_relations=False,
                generate_embedding=False, extraction_mode="skip",
            )
            isolated = await client.short_term.get_conversation(session)
            other = await client.short_term.get_conversation("muninn-oracle-other")
            verify_sessions(
                [isolated.model_dump(mode="json"), other.model_dump(mode="json")],
                [conversation.model_dump(mode="json"), {
                    "id": str(other_message.conversation_id),
                    "messages": [other_message.model_dump(mode="json")],
                }],
            )
            deleted, error_code = None, None
            try:
                deleted = await client.short_term.delete_message(messages[1].id)
            except ConstraintError as error:
                error_code = error.code
            remaining = await client.short_term.get_conversation(session)
            snapshot = {
                "before_delete": [normalize_message(m.model_dump(mode="json")) for m in conversation.messages],
                "after_delete": [normalize_message(m.model_dump(mode="json")) for m in remaining.messages],
                "delete_message": deletion_observation(
                    deleted, error_code, [m.content for m in remaining.messages], [contents[0], contents[2]]),
                "session_isolation": {"checked_sessions": 2, "verdict": "PASS"},
            }
        queries = {
            "neo4j": "CALL dbms.components() YIELD versions, edition RETURN versions, edition",
            "relationships": "MATCH ()-[r]->() RETURN type(r) AS type, count(*) AS count ORDER BY type",
            "constraints": "SHOW CONSTRAINTS YIELD name, type, entityType, labelsOrTypes, properties RETURN * ORDER BY name",
            "indexes": "SHOW INDEXES YIELD name, type, entityType, labelsOrTypes, properties, options RETURN * ORDER BY name",
        }
        for name, query in queries.items():
            records, _, _ = await driver.execute_query(query, database_="neo4j")
            snapshot[name] = [record.data() for record in records]
        return snapshot


def main():
    from reference import materialize_sources

    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--sources", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    base = Path(__file__).resolve().parent
    try:
        references = json.loads((base / "references.json").read_text(encoding="utf-8"))
        integrity = json.loads((base / "source-integrity.json").read_text(encoding="utf-8"))
        if {name: item["commit"] for name, item in integrity.items()} != {
            name: item["commit"] for name, item in references.items()
        }:
            raise ValueError("Los manifiestos no fijan los mismos commits")
        with TemporaryDirectory(prefix="muninn-oracle-") as execution:
            clean = Path(execution)
            materialize_sources(args.sources, integrity, clean)
            # Solo esta copia limpia entra en sys.path, nunca el src original.
            sys.path.insert(0, str(clean / "agent-memory/src"))
            auth = Path("/run/secrets/neo4j_auth").read_text(encoding="utf-8").strip()
            username, password = auth.split("/", 1)
            if username != "neo4j" or not password:
                raise ValueError("Secreto de prueba inválido")
            snapshot = asyncio.run(capture(password))
        snapshot["reference_commits"] = {name: item["commit"] for name, item in references.items()}
        snapshot["runtime"] = {
            "python": platform.python_version(),
            "packages": {name: version(name) for name in (
                "neo4j", "pydantic", "pydantic-core", "pydantic-settings")},
        }
        snapshot["normalization"] = "Se omiten id, conversation_id, timestamp, created_at y updated_at solo del primer nivel de Message"
        snapshot["scope"] = "oraculo_sin_extraccion_ni_embeddings; no demuestra paridad Muninn"
        args.output.parent.mkdir(parents=True, exist_ok=True)
        args.output.write_text(json.dumps(snapshot, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    except Exception as error:
        # Las excepciones de proveedores pueden contener entradas sensibles.
        print(f"Captura fallida ({type(error).__name__}); no se genera evidencia de éxito", file=sys.stderr)
        return 1
    if snapshot["delete_message"]["normative_verdict"] != "PASS":
        print("Captura guardada con discrepancia normativa en delete_message; no demuestra paridad", file=sys.stderr)
        return 2
    print("Captura del oráculo verificada; sin ejecución TCK ni comparación Muninn")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
