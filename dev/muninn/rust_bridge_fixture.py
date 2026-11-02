"""Solo autentica el transporte TCK; la memoria se ejecuta en Rust."""

import asyncio
from pathlib import Path
import time

import httpx
import pytest_asyncio

from tck.adapters.http_bridge import HTTPBridgeAdapter, _serialize


EVENTS = []


class AuthenticatedBridge(HTTPBridgeAdapter):
    def __init__(self):
        super().__init__("http://bridge:8080", timeout=12)
        self._fixture_token = Path("/run/secrets/bridge_token").read_text().strip()

    async def _call(self, method, params=None):
        body = {key: _serialize(value) for key, value in (params or {}).items() if value is not None}
        async with httpx.AsyncClient(base_url=self._base_url, timeout=self._timeout, trust_env=False) as client:
            response = await client.post("/" + method, json=body,
                                         headers={"Authorization": "Bearer " + self._fixture_token})
        event = {"method": method, "params": body, "status": response.status_code}
        if response.content:
            event["result"] = response.json()
        EVENTS.append(event)
        if response.status_code >= 400:
            raise RuntimeError(f"Rust bridge {method} HTTP {response.status_code}")
        return event.get("result")


async def ready_bridge():
    instance = AuthenticatedBridge()
    deadline = time.monotonic() + 30
    while True:
        try:
            await instance.setup()
            return instance
        except httpx.TransportError:
            if time.monotonic() >= deadline:
                raise
            await asyncio.sleep(0.1)


@pytest_asyncio.fixture(scope="session", loop_scope="session")
async def adapter():
    instance = await ready_bridge()
    try:
        yield instance
    finally:
        await instance.teardown()
