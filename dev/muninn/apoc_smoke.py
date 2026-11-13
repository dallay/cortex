"""Verifica apoc.version en una base sintética; no prueba todas las funciones."""

import argparse
import asyncio
import json
from pathlib import Path
import sys


def valid_result(rows, expected):
    return len(rows) == 1 and rows[0].get("version") == expected


async def probe(password):
    from neo4j import AsyncGraphDatabase

    async with AsyncGraphDatabase.driver("bolt://neo4j:7687", auth=("neo4j", password)) as driver:
        versions, _, _ = await driver.execute_query(
            "CALL dbms.components() YIELD versions,edition RETURN versions,edition", database_="neo4j")
        rows, _, _ = await driver.execute_query("RETURN apoc.version() AS version", database_="neo4j")
        return [r.data() for r in versions], [r.data() for r in rows]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--manifest", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    try:
        if args.output.exists():
            raise ValueError("La salida debe ser nueva")
        plugin = json.loads(args.manifest.read_text())["plugins"][0]
        auth = Path("/run/secrets/neo4j_auth").read_text().strip().split("/",1)
        if len(auth) != 2 or auth[0] != "neo4j":
            raise ValueError("Credencial de fixture inválida")
        versions, result = asyncio.run(probe(auth[1]))
        if not valid_result(result, plugin["version_from_filename"]):
            raise ValueError("Versión del plugin no coincide")
        receipt = {"scope":"Solo apoc.version; sin pruebas de otras funciones ni paridad Muninn",
                   "image":plugin["image"],"jar_sha256":plugin["sha256"],
                   "neo4j":versions,"query":"RETURN apoc.version() AS version",
                   "actual":result,"verdict":"PASS"}
        args.output.parent.mkdir(parents=True,exist_ok=True)
        args.output.write_text(json.dumps(receipt,ensure_ascii=False,indent=2)+'\n')
    except Exception as error:
        print(f"APOC bloqueado ({type(error).__name__}); no se publica aprobación",file=sys.stderr)
        return 1
    print("APOC version verificado en Neo4j real; no equivale a compatibilidad completa")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
