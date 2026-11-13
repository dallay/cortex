"""Superficies fijadas y límites de red del discovery MCP del oráculo."""

from threading import RLock


CORE_TOOLS = {
    "memory_search", "memory_get_context", "memory_store_message",
    "memory_add_entity", "memory_add_preference", "memory_add_fact",
}
EXTENDED_TOOLS = {
    "memory_get_conversation", "memory_list_sessions", "memory_get_entity",
    "memory_export_graph", "memory_create_relationship", "memory_start_trace",
    "memory_record_step", "memory_complete_trace", "memory_get_observations", "graph_query",
}
PLATINUM_TOOLS = {
    "memory_set_entity_feedback", "memory_get_entity_history",
    "memory_get_entity_provenance", "memory_get_reflections",
}


def expected_surface(profile, backend):
    if profile not in {"core", "extended"} or backend not in {"none", "nams"}:
        raise ValueError("Perfil o backend fuera de la captura autorizada")
    tools = set(CORE_TOOLS)
    resources = set()
    prompts = {"memory-conversation"}
    if profile == "extended":
        tools |= EXTENDED_TOOLS
        resources = {"memory://entities", "memory://preferences", "memory://graph/stats"}
        prompts |= {"memory-reasoning", "memory-review"}
        if backend == "nams":
            tools |= PLATINUM_TOOLS
    return {"tools": tools, "resources": resources,
            "templates": {"memory://context/{session_id}"}, "prompts": prompts}


def validate_surface(payload, profile, backend):
    """No permite cobertura parcial, perfiles mezclados ni páginas omitidas."""
    expected = expected_surface(profile, backend)
    for kind, collection, identity in (
        ("tools", "tools", "name"), ("resources", "resources", "uri"),
        ("templates", "resourceTemplates", "uriTemplate"), ("prompts", "prompts", "name"),
    ):
        if collection not in payload or collection not in payload[collection]:
            raise ValueError(f"Respuesta ausente en {kind}")
        envelope = payload.get(collection, {})
        if envelope.get("nextCursor"):
            raise ValueError(f"Página adicional sin consumir en {kind}")
        if envelope.get("resultType", "complete") != "complete":
            raise ValueError(f"Resultado incompleto en {kind}")
        items = envelope.get(collection, [])
        identifiers = [item[identity] for item in items]
        if len(identifiers) != len(set(identifiers)) or set(identifiers) != expected[kind]:
            raise ValueError(f"Superficie incompleta o inesperada: {kind}")
    for tool in payload["tools"]["tools"]:
        if tool.get("inputSchema", {}).get("type") != "object":
            raise ValueError("Esquema MCP ausente o incompatible")


class NetworkPolicyError(PermissionError):
    """Denegación identificable en la cadena causal del fallo del probe."""


class NetworkAudit:
    """Bloquea cualquier conexión distinta del MCP loopback de esta captura."""

    def __init__(self, sink=None, role="client"):
        if role not in {"client", "server-stdio", "server-http"}:
            raise ValueError("Rol de red no autorizado")
        self.counters = {"connects_allowed": 0, "dns_allowed": 0, "binds_allowed": 0, "blocked": 0}
        self.sink = sink
        self.lock = RLock()
        self.role = role
        self.phase = "prepare"
        self.denials = []

    def __call__(self, event, args):
        extra = {"socket.sendto", "socket.sendmsg", "socket.gethostbyname",
                 "socket.gethostbyname_ex", "socket.gethostbyaddr", "socket.getnameinfo"}
        if event not in {"socket.connect", "socket.bind", "socket.getaddrinfo"} | extra:
            return
        if event == "socket.getaddrinfo":
            address = args[:2]
        elif event.startswith("socket.gethost"):
            address = (args[0], None)
        else:
            address = args[-1]
        host, port = address[:2] if isinstance(address, tuple) and len(address) >= 2 else (None, None)
        local = host in {"127.0.0.1", "::1", "localhost"} and port == 8765
        allowed = local and (
            (self.role == "client" and event in {"socket.connect", "socket.getaddrinfo"})
            or (self.role == "server-http" and event == "socket.bind" and host == "127.0.0.1"))
        with self.lock:
            key = {"socket.connect": "connects_allowed", "socket.bind": "binds_allowed",
                   "socket.getaddrinfo": "dns_allowed"}[event] if allowed else "blocked"
            self.counters[key] += 1
            if not allowed and len(self.denials) < 64:
                self.denials.append({"event": event, "host": host if host in
                                     {"127.0.0.1", "::1", "localhost"} else "<nonlocal>",
                                     "port": port, "phase": self.phase})
            if self.sink is not None:
                self.sink(self.snapshot())
        if not allowed:
            raise NetworkPolicyError("Red fuera de la política del proceso bloqueada")

    def snapshot(self):
        with self.lock:
            return {**self.counters, "role": self.role, "phase": self.phase,
                    "denials": [dict(item) for item in self.denials]}
