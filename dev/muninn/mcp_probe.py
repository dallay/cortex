"""Discovery MCP real del oráculo fijado; no implementa ni certifica Muninn."""

import argparse
import asyncio
from contextlib import contextmanager
import hashlib
from importlib.metadata import version
import json
import os
from pathlib import Path
import platform
import secrets
import sys
from tempfile import TemporaryDirectory

from mcp_contract import NetworkAudit, NetworkPolicyError, validate_surface
from reference import materialize_sources


def wire(model):
    return model.model_dump(mode="json", by_alias=True, exclude_none=True)


@contextmanager
def verified_sources(root):
    base = Path(__file__).resolve().parent
    references = json.loads((base / "references.json").read_text(encoding="utf-8"))
    integrity = json.loads((base / "source-integrity.json").read_text(encoding="utf-8"))
    if {name: item["commit"] for name, item in references.items()} != {
        name: item["commit"] for name, item in integrity.items()
    }:
        raise ValueError("Commits incompatibles entre manifiestos")
    with TemporaryDirectory(prefix="muninn-mcp-") as execution:
        clean = Path(execution) / "sources"
        materialize_sources(root, integrity, clean)
        sys.path.insert(0, str(clean / "agent-memory/src"))
        yield clean, references, Path(execution)


def configure_environment(home, canary):
    path = os.environ.get("PATH", "")
    os.environ.clear()
    os.environ.update({"PATH": path, "HOME": str(home), "LANG": "C.UTF-8",
                       "PYTHONDONTWRITEBYTECODE": "1", "MUNINN_MCP_CANARY": canary})


def create_server(profile, backend, canary, validate_connect=False):
    from neo4j_agent_memory.config.settings import MemorySettings
    from neo4j_agent_memory.mcp.server import create_mcp_server

    settings = None
    if backend == "nams":
        settings = MemorySettings(
            _env_file=None, backend="nams", nams={
                "endpoint": "http://127.0.0.1:9/v1", "api_key": canary,
                "validate_on_connect": validate_connect, "max_retries": 0, "timeout": 0.25,
            },
        )
    return create_mcp_server(settings=settings, profile=profile)


async def registry_capture(profile, backend, canary):
    server = create_server(profile, backend, canary)
    result = {
        "tools": {"tools": [wire(item.to_mcp_tool()) for item in await server.list_tools()]},
        "resources": {"resources": [wire(item.to_mcp_resource()) for item in await server.list_resources()]},
        "resourceTemplates": {"resourceTemplates": [wire(item.to_mcp_template()) for item in await server.list_resource_templates()]},
        "prompts": {"prompts": [wire(item.to_mcp_prompt()) for item in await server.list_prompts()]},
    }
    validate_surface(result, profile, backend)
    return result


async def protocol_error(client, name, arguments):
    from mcp.shared.exceptions import MCPError

    try:
        result = wire(await client.call_tool_mcp(name, arguments))
    except MCPError as error:
        return {"kind": "protocol_error", "error": wire(error.error)}
    if result.get("isError") is not True:
        raise ValueError("La llamada negativa no devolvió un error MCP")
    return {"kind": "tool_error", "result": result}


async def session_capture(client, profile, backend):
    async with client:
        result = {
            "tools": wire(await client.list_tools_mcp()),
            "resources": wire(await client.list_resources_mcp()),
            "resourceTemplates": wire(await client.list_resource_templates_mcp()),
            "prompts": wire(await client.list_prompts_mcp()),
        }
        validate_surface(result, profile, backend)
        result["negotiated"] = {
            "protocolVersion": client.protocol_version,
            "serverInfo": wire(client.server_info),
            "capabilities": wire(client.server_capabilities),
            "instructions": client.instructions,
        }
        if not client.protocol_version:
            raise ValueError("No se negoció una versión del protocolo MCP")
        result["unknown_tool"] = await protocol_error(client, "muninn_missing_tool", {})
        if backend == "none":
            result["no_backend_call"] = await protocol_error(
                client, "memory_get_context", {"query": "muninn-discovery-synthetic"})
        return result


async def stop_process(process, grace_seconds=3):
    forced_kill = False
    if process.returncode is None:
        try:
            process.terminate()
        except ProcessLookupError:
            pass
        try:
            await asyncio.wait_for(process.wait(), timeout=grace_seconds)
        except TimeoutError:
            forced_kill = True
            process.kill()
            await process.wait()
    return {"return_code": process.returncode, "forced_kill": forced_kill}


def caused_by_policy(error):
    pending, seen = [error], set()
    while pending:
        current = pending.pop()
        if current is None or id(current) in seen:
            continue
        seen.add(id(current))
        if isinstance(current, NetworkPolicyError):
            return True
        pending.extend([current.__cause__, current.__context__, *getattr(current, "exceptions", ())])
    return False


def validate_child_audit(audit, transport):
    if (audit.get("blocked") != 0 or audit.get("connects_allowed") != 0 or audit.get("dns_allowed") != 0
            or audit.get("binds_allowed") != (1 if transport == "http" else 0)
            or audit.get("role") != f"server-{transport}"):
        raise ValueError("La red del hijo no corresponde a su rol de discovery")


def validate_probe_failure(audit):
    failure = audit.get("startup_failure") or {}
    denials = audit.get("denials", [])
    if (audit.get("backend") != "nams" or audit.get("validate_on_connect") is not True
            or audit.get("phase") != "nams_probe" or audit.get("role") != "server-stdio"
            or audit.get("finished") is not True or audit.get("exit_code") != 1
            or failure.get("type") != "TransportError" or failure.get("caused_by_policy") is not True
            or audit.get("blocked") != 1 or len(denials) != 1
            or any(audit.get(key) != 0 for key in ("connects_allowed", "dns_allowed", "binds_allowed"))):
        raise ValueError("Fallo del probe sin atribución suficiente")
    denied = denials[0]
    if (denied.get("event") not in {"socket.connect", "socket.getaddrinfo"}
            or denied.get("host") != "127.0.0.1" or denied.get("port") != 9
            or denied.get("phase") != "nams_probe"):
        raise ValueError("El bloqueo no corresponde al endpoint del probe NAMS")


async def completed_receipt(path):
    async with asyncio.timeout(5):
        while True:
            if path.is_file():
                receipt = json.loads(path.read_text(encoding="utf-8"))
                if receipt.get("finished") is True:
                    return receipt
            await asyncio.sleep(0.05)


async def wait_for_http(process):
    async with asyncio.timeout(20):
        while True:
            if process.returncode is not None:
                raise RuntimeError("El servidor HTTP terminó antes del discovery")
            try:
                _, writer = await asyncio.open_connection("127.0.0.1", 8765)
                writer.close()
                await writer.wait_closed()
                return
            except OSError:
                await asyncio.sleep(0.05)


async def transport_capture(clean, output, profile, backend, transport, validate_connect=False):
    from fastmcp import Client
    from fastmcp.client.transports import StdioTransport

    label = f"{backend}-{profile}-{transport}" + ("-probe" if validate_connect else "")
    audit_path = output / f"{label}.audit.json"
    log_path = output / f"{label}.stderr.log"
    base = Path(__file__).resolve().parent
    command = [sys.executable, str(base / "mcp_probe.py"), "--serve", "--sources", str(clean),
               "--profile", profile, "--backend", backend, "--transport", transport,
               "--audit-file", str(audit_path)]
    if validate_connect:
        command += ["--validate-connect"]
    if transport == "stdio":
        link = StdioTransport(command=command[0], args=command[1:], env=dict(os.environ),
                              cwd=str(base), keep_alive=False, log_file=log_path)
        result = await session_capture(Client(link, timeout=10, init_timeout=10), profile, backend)
        receipt = await completed_receipt(audit_path)
        if receipt.get("exit_code") != 0:
            raise ValueError("El hijo stdio no completó su cierre")
        return result, audit_path, {"mechanism": "server_finally_receipt", "return_code": 0}
    with log_path.open("w", encoding="utf-8") as log:
        process = await asyncio.create_subprocess_exec(*command, stdout=log, stderr=log, env=dict(os.environ))
        try:
            await wait_for_http(process)
            result = await session_capture(
                Client("http://127.0.0.1:8765/mcp", timeout=10, init_timeout=10), profile, backend)
        finally:
            shutdown = await stop_process(process)
    if shutdown["forced_kill"] or shutdown["return_code"] not in {0, -15}:
        raise ValueError("El cierre HTTP requirió SIGKILL o terminó de forma inesperada")
    return result, audit_path, shutdown


def comparable(payload):
    return {key: sorted(payload[key][key], key=lambda item: json.dumps(item, sort_keys=True))
            for key in ("tools", "resources", "resourceTemplates", "prompts")}


def publish_snapshot(output, snapshot, canary):
    """Publica el recibo final solo tras verificar todos los artefactos."""
    content = json.dumps(snapshot, ensure_ascii=False, indent=2) + "\n"
    if canary in content or any(canary in path.read_text(encoding="utf-8")
                               for path in output.iterdir() if path.is_file()):
        raise ValueError("El canary sintético apareció en la evidencia")
    temporary = output / "discovery.pending"
    temporary.write_text(content, encoding="utf-8")
    temporary.replace(output / "discovery.json")


async def capture_matrix(clean, output, canary):
    matrix = []
    for backend in ("none", "nams"):
        for profile in ("core", "extended"):
            registry = await registry_capture(profile, backend, canary)
            entry = {"backend": backend, "profile": profile, "registry": registry, "sessions": {}}
            for transport in ("stdio", "http"):
                session, audit_path, shutdown = await transport_capture(clean, output, profile, backend, transport)
                audit = json.loads(audit_path.read_text(encoding="utf-8"))
                validate_child_audit(audit, transport)
                if comparable(session) != comparable(registry):
                    raise ValueError("El registro y el discovery del protocolo no coinciden")
                entry["sessions"][transport] = {"discovery": session, "network_audit": audit, "shutdown": shutdown}
            matrix.append(entry)
    # Control negativo del probe habilitado, con destino sintético y sin servicio real.
    expected_audit = output / "nams-core-stdio-probe.audit.json"
    try:
        await transport_capture(clean, output, "core", "nams", "stdio", validate_connect=True)
    except Exception:
        audit = await completed_receipt(expected_audit)
        validate_probe_failure(audit)
    else:
        raise ValueError("El probe NAMS no fue bloqueado")
    return matrix, {"validate_on_connect": True, "outcome": "BLOCKED_AS_EXPECTED", "network_audit": audit}


async def serve(args, canary, audit):
    server = create_server(args.profile, args.backend, canary, args.validate_connect)
    audit.phase = "nams_probe" if args.backend == "nams" and args.validate_connect else "serve"
    if args.transport == "stdio":
        await server.run_async(transport="stdio", show_banner=False)
    else:
        await server.run_async(transport="http", host="127.0.0.1", port=8765, show_banner=False)


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--sources", type=Path, required=True)
    parser.add_argument("--output", type=Path)
    parser.add_argument("--serve", action="store_true")
    parser.add_argument("--profile", choices=("core", "extended"), default="core")
    parser.add_argument("--backend", choices=("none", "nams"), default="none")
    parser.add_argument("--transport", choices=("stdio", "http"), default="stdio")
    parser.add_argument("--audit-file", type=Path)
    parser.add_argument("--validate-connect", action="store_true")
    args = parser.parse_args(argv)
    if (args.serve and args.audit_file is None) or (not args.serve and args.output is None):
        parser.error("Se requiere --audit-file para el servidor o --output para la captura")
    startup_failure = None
    finished = False
    exit_code = None

    def persist_audit(snapshot):
        temporary = args.audit_file.with_suffix(".pending")
        receipt = {**snapshot, "profile": args.profile, "backend": args.backend,
                   "validate_on_connect": args.validate_connect, "startup_failure": startup_failure,
                   "finished": finished, "exit_code": exit_code}
        temporary.write_text(json.dumps(receipt) + "\n", encoding="utf-8")
        temporary.replace(args.audit_file)

    audit = NetworkAudit(sink=persist_audit if args.serve else None,
                         role=f"server-{args.transport}" if args.serve else "client")
    if args.serve:
        persist_audit(audit.snapshot())
    sys.addaudithook(audit)
    canary = os.environ.get("MUNINN_MCP_CANARY") if args.serve else None
    canary = canary or secrets.token_hex(24)
    try:
        with verified_sources(args.sources) as (clean, references, execution):
            home = execution / "home"
            home.mkdir()
            configure_environment(home, canary)
            if args.serve:
                asyncio.run(serve(args, canary, audit))
                exit_code = 0
                return 0
            output = args.output.resolve()
            output.mkdir(parents=True, exist_ok=False)
            matrix, negative = asyncio.run(capture_matrix(clean, output, canary))
            if audit.snapshot()["blocked"] != 0:
                raise ValueError("El cliente intentó una conexión no autorizada")
            from neo4j_agent_memory.mcp.server import normalize_transport

            aliases = {name: normalize_transport(name) for name in ("stdio", "http", "streamable-http", "sse")}
            snapshot = {
                "scope": "Discovery del oráculo, no conformidad de Muninn ni validación NAMS/persistencia",
                "reference_commits": {name: item["commit"] for name, item in references.items()},
                "runtime": {"python": platform.python_version(), "packages": {
                    name: version(name) for name in ("fastmcp", "fastmcp-slim", "mcp", "mcp-types", "httpx", "httpx2")}},
                "local_files_sha256": {name: hashlib.sha256((Path(__file__).parent / name).read_bytes()).hexdigest()
                                       for name in ("mcp_probe.py", "mcp_contract.py", "reference.py", "requirements-mcp.txt",
                                                    "references.json", "source-integrity.json", "Dockerfile.mcp", "compose.mcp.yaml")},
                "transport_aliases": aliases, "matrix": matrix, "nams_default_probe": negative,
                "parent_network_audit": audit.snapshot(),
                "comparison": "Se comparan los elementos completos de cada lista, ignorando su orden; los metadatos del envoltorio de protocolo no tienen equivalente en el registro y se conservan sin compararlos",
            }
            publish_snapshot(output, snapshot, canary)
    except Exception as error:
        startup_failure = {"type": type(error).__name__, "caused_by_policy": caused_by_policy(error)}
        exit_code = 1
        print(f"Discovery bloqueado ({type(error).__name__}); no se declara completado", file=sys.stderr)
        return 1
    finally:
        if args.serve:
            finished = True
            persist_audit(audit.snapshot())
    print("Discovery capturado: cuatro perfiles/backend, ocho sesiones MCP; sin conformidad de Muninn")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
