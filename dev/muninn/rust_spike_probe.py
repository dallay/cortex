"""Verificaciones independientes del TCK: grafo, reinicio y denegaciones."""

import argparse
import asyncio
import fcntl
import hashlib
import json
from pathlib import Path
import sys


SCOPE = "muninn-rust-spike"


def valid_persistence(expected, graph):
    if not graph.get("owner_valid") or any(graph.get(key) != 1 for key in
                                          ("conversations", "messages", "has_message", "first_message")):
        return False
    rows = graph.get("rows", [])
    if len(rows) != 1:
        return False
    row = rows[0]
    return (all(row.get(key) == expected[key] for key in
                ("session_id", "conversation_id", "message_id", "content"))
            and row.get("role") == "user" and json.loads(row.get("metadata", "null")) == {})


class ProbeError(Exception):
    pass


async def request(token, method, body, expected_status):
    import httpx

    headers = {"Authorization":"Bearer "+token} if token else {}
    async with httpx.AsyncClient(trust_env=False,timeout=12) as client:
        response=await client.post("http://bridge:8080/"+method,json=body,headers=headers)
    if response.status_code != expected_status:
        raise ProbeError(f"{method}: HTTP {response.status_code}, esperado {expected_status}")
    return response.json()


async def snapshot(driver, owner):
    async def one(query, **parameters):
        rows,_,_=await driver.execute_query(query,parameters_=parameters,database_="neo4j")
        return [r.data() for r in rows]

    guard=await one("MATCH (g:MuninnBridgeGuard {scope:$scope}) RETURN count(g) AS total,sum(CASE WHEN g.owner=$owner THEN 1 ELSE 0 END) AS owned",scope=SCOPE,owner=owner)
    rows=await one("MATCH (c:Conversation {scope:$scope})-[:HAS_MESSAGE]->(m:Message {scope:$scope}) RETURN c.id AS conversation_id,c.session_id AS session_id,m.id AS message_id,m.role AS role,m.content AS content,m.metadata AS metadata",scope=SCOPE)
    versions=await one("CALL dbms.components() YIELD versions,edition RETURN versions,edition")
    result={"owner_valid":guard==[{"total":1,"owned":1}],"rows":rows,"neo4j":versions}
    for key,statement in (
        ("conversations","MATCH (n:Conversation {scope:$scope}) RETURN count(n) AS count"),
        ("messages","MATCH (n:Message {scope:$scope}) RETURN count(n) AS count"),
        ("has_message","MATCH (:Conversation {scope:$scope})-[r:HAS_MESSAGE]->(:Message {scope:$scope}) RETURN count(r) AS count"),
        ("first_message","MATCH (:Conversation {scope:$scope})-[r:FIRST_MESSAGE]->(:Message {scope:$scope}) RETURN count(r) AS count"),
    ):
        result[key]=(await one(statement,scope=SCOPE))[0]["count"]
    return result


def expected_from(receipt):
    adds=[e for e in receipt["events"] if e["method"]=="add_message" and e["status"]==200]
    gets=[e for e in receipt["events"] if e["method"]=="get_conversation" and e["status"]==200]
    if receipt["verdict"] != "PASS" or receipt["counts"]["passed"] != 1 or len(adds)!=1 or len(gets)!=1:
        raise ProbeError("No existe el Bronze exacto esperado")
    return {"session_id":adds[0]["params"]["session_id"],"conversation_id":gets[0]["result"]["id"],
            "message_id":adds[0]["result"]["id"],"content":adds[0]["params"]["content"]}


async def run(mode, expected, token, password):
    if mode=="unavailable":
        result=await request(token,"get_conversation",{"session_id":expected["session_id"]},503)
        if result.get("error")!="storage_unavailable":raise ProbeError("No se informó caída de almacenamiento")
        return {"verdict":"PASS","scope":"Solo fallo de Neo4j","status":503}
    from neo4j import AsyncGraphDatabase

    owner=hashlib.sha256(token.encode()).hexdigest()
    async with AsyncGraphDatabase.driver("bolt://neo4j:7687",auth=("neo4j",password)) as driver:
        graph=await snapshot(driver,owner)
        if not valid_persistence(expected,graph):raise ProbeError("El grafo no coincide con respuestas Rust")
        if mode=="recover":
            actual=await request(token,"get_conversation",{"session_id":expected["session_id"]},200)
            if actual["id"]!=expected["conversation_id"] or [m["id"] for m in actual["messages"]]!=[expected["message_id"]]:
                raise ProbeError("El reinicio Rust perdió identidades o datos")
            return {"verdict":"PASS","scope":"Lectura después de reiniciar solo Rust","expected":expected,"graph":graph}
        if mode=="security":
            for method in ("setup","clear_all_data","add_message","get_conversation"):
                await request(None,method,{},401)
            await request("invalid","clear_all_data",{},401)
            await request(token,"get_conversation",{"session_id":expected["session_id"],"scope":"other"},400)
            await request(token,"add_message",{"session_id":"canary","role":"user","content":token},400)
            await request(token,"get_conversation",{"session_id":token},400)
            await request(token,"get_conversation",{"session_id":password},400)
            if not valid_persistence(expected,await snapshot(driver,owner)):
                raise ProbeError("Las peticiones rechazadas alteraron la memoria")
            await driver.execute_query("MATCH (g:MuninnBridgeGuard {scope:$scope}) SET g.owner='revoked'",scope=SCOPE,database_="neo4j")
            try:
                await request(token,"clear_all_data",{},409)
                await request(token,"get_conversation",{"session_id":expected["session_id"]},409)
            finally:
                await driver.execute_query("MATCH (g:MuninnBridgeGuard {scope:$scope}) SET g.owner=$owner",scope=SCOPE,owner=owner,database_="neo4j")
            if not valid_persistence(expected,await snapshot(driver,owner)):
                raise ProbeError("La limpieza sin propiedad alteró los datos")
            await driver.execute_query("CREATE (:FixtureForeign {scope:'muninn-other-fixture'})",database_="neo4j")
            await request(token,"clear_all_data",{},200)
            own=await snapshot(driver,owner)
            foreign,_,_=await driver.execute_query("MATCH (f:FixtureForeign {scope:'muninn-other-fixture'}) RETURN count(f) AS count",database_="neo4j")
            if own["messages"]!=0 or own["conversations"]!=0 or foreign[0]["count"]!=1:
                raise ProbeError("La limpieza no respetó el scope")
            return {"verdict":"PASS","scope":"Controles del spike, no seguridad integral","unauthorized":"401","scope_spoof":"400","secret_payload":"400","ownership_loss":"409","scoped_cleanup":"foreign_preserved"}
        return {"verdict":"PASS","scope":"Inspección directa del grafo creado por Rust","expected":expected,"graph":graph}


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--mode',choices=('inspect','recover','security','unavailable'),required=True)
    parser.add_argument('--receipt',type=Path,required=True)
    parser.add_argument('--output',type=Path,required=True)
    args=parser.parse_args()
    try:
        if args.output.exists():raise ProbeError('La salida debe ser nueva')
        with Path('/artifacts/.rust-bridge.lock').open('a') as lease:
            fcntl.flock(lease,fcntl.LOCK_EX|fcntl.LOCK_NB)
            expected=expected_from(json.loads(args.receipt.read_text()))
            token=Path('/run/secrets/bridge_token').read_text().strip()
            password=Path('/run/secrets/neo4j_auth').read_text().strip().split('/',1)[1]
            result=asyncio.run(run(args.mode,expected,token,password))
            content=json.dumps(result,ensure_ascii=False,indent=2)+'\n'
            if token in content or password in content:raise ProbeError('Canary de servidor en evidencia')
            args.output.parent.mkdir(parents=True,exist_ok=True);args.output.write_text(content)
    except ProbeError as error:
        print('Prueba Rust bloqueada: '+str(error),file=sys.stderr);return 1
    except Exception as error:
        print(f'Prueba Rust bloqueada ({type(error).__name__})',file=sys.stderr);return 1
    print('Verificación Rust '+args.mode+': PASS (solo este corte)');return 0


if __name__=='__main__':raise SystemExit(main())
