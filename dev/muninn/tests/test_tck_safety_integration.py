"""Regresión concurrente opt-in, exclusiva del contenedor TCK efímero."""

import asyncio
import os
from pathlib import Path
import sys
import unittest
from unittest.mock import patch


@unittest.skipUnless(os.getenv("MUNINN_TCK_SAFETY_INTEGRATION") == "1",
                     "Requiere el contenedor TCK efímero, nunca una base persistente")
class OwnershipIntegrationTests(unittest.IsolatedAsyncioTestCase):
    async def test_only_one_concurrent_owner_can_claim_the_empty_database(self):
        from neo4j import AsyncGraphDatabase

        sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
        import tck_runner

        password = Path("/run/secrets/neo4j_auth").read_text().strip().split("/", 1)[1]
        async with AsyncGraphDatabase.driver("bolt://neo4j:7687", auth=("neo4j", password)) as driver:
            records, _, _ = await driver.execute_query("MATCH (n) RETURN count(n) AS count", database_="neo4j")
            self.assertEqual(records[0]["count"], 0, "La prueba exige una base sintética vacía")
        barrier = asyncio.Barrier(2)
        original_claim = tck_runner.claim_transaction

        class CachedResult:
            def __init__(self, record):
                self.record = record

            async def single(self):
                return self.record

        class SynchronizedTransaction:
            def __init__(self, transaction):
                self.transaction = transaction

            async def run(self, query, **parameters):
                result = await self.transaction.run(query, **parameters)
                if query == "MATCH (n) RETURN count(n) AS count":
                    record = await result.single()
                    await barrier.wait()
                    return CachedResult(record)
                return result

        async def synchronized_claim(transaction, token):
            return await original_claim(SynchronizedTransaction(transaction), token)

        with patch.object(tck_runner, "claim_transaction", synchronized_claim):
            results = await asyncio.wait_for(asyncio.gather(
                tck_runner.claim_database(password, "fixture-owner-a"),
                tck_runner.claim_database(password, "fixture-owner-b"),
                return_exceptions=True), timeout=60)
        self.assertEqual(sum(not isinstance(r, Exception) for r in results), 1,
                         "Dos ejecuciones obtuvieron propiedad sobre la misma base")
        async with AsyncGraphDatabase.driver("bolt://neo4j:7687", auth=("neo4j", password)) as driver:
            records, _, _ = await driver.execute_query(
                "MATCH (g:MuninnTckGuard) RETURN count(g) AS count, collect(g.token) AS tokens", database_="neo4j")
            self.assertEqual(records[0]["count"], 1)
            token = records[0]["tokens"][0]
            await driver.execute_query(
                "CREATE (:MuninnTckGuard {name:'unexpected', token:'unexpected'})", database_="neo4j")

            async def execute(query, parameters):
                rows, _, _ = await driver.execute_query(query, parameters_=parameters, database_="neo4j")
                return [row.data() for row in rows]

            with self.assertRaisesRegex(ValueError, "marca"):
                await tck_runner.guarded_reset(execute, execute, token)
            rows = await execute("MATCH (g:MuninnTckGuard) RETURN count(g) AS count", {})
            self.assertEqual(rows[0]["count"], 2, "El reset no debe borrar otra marca")


if __name__ == "__main__":
    unittest.main()
