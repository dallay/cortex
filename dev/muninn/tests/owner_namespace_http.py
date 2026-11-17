#!/usr/bin/env python3
"""Opt-in HTTP checks against an explicitly configured synthetic fixture.

Creates UUID-isolated data. Never point this script at an active database.
Uses no admin reset and requires no third-party packages.
"""
import argparse
import json
import urllib.error
import urllib.request
import uuid


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--url", required=True)
    args = parser.parse_args()
    base = args.url.rstrip("/")
    if not base.startswith("http://127.0.0.1:"):
        parser.error("only an explicit loopback synthetic bridge is permitted")

    def call(method, payload, expected=200):
        request = urllib.request.Request(
            f"{base}/{method}", json.dumps(payload).encode(),
            {"Content-Type": "application/json"}, method="POST",
        )
        try:
            with urllib.request.urlopen(request, timeout=20) as response:
                status, body = response.status, response.read()
        except urllib.error.HTTPError as error:
            status, body = error.code, error.read()
        assert status == expected, (method, status, expected, body.decode())
        return json.loads(body) if body else None

    session = f"http-namespace-{uuid.uuid4()}"
    owner_a, owner_b = f"a-{uuid.uuid4()}", f"b-{uuid.uuid4()}"
    resources = []
    for owner in (owner_a, owner_b, None):
        scope = {} if owner is None else {"user_identifier": owner}
        message = call("add_message", {
            "session_id": session, "role": "user", "content": "same-content",
            **scope,
        })
        conversation = call("get_conversation", {"session_id": session, **scope})
        assert len(conversation["messages"]) == 1
        resources.append((scope, message["id"], conversation["id"]))
    assert len({item[2] for item in resources}) == 3
    scope_a, message_a, conversation_a = resources[0]
    scope_b = resources[1][0]
    call("delete_message", {"message_id": message_a, **scope_b})
    call("delete_conversation", {"conversation_id": conversation_a, **scope_b}, 204)
    own = call("get_conversation", {"session_id": session, **scope_a})
    assert [message["id"] for message in own["messages"]] == [message_a]
    listed = call("list_conversations", {"limit": 100, **scope_a})
    assert any(item["id"] == conversation_a for item in listed)
    assert all(item["id"] not in {resources[1][2], resources[2][2]} for item in listed)
    second = call("add_message", {"session_id": session, "role": "user", "content": "same-content", **scope_a})
    assert second["id"] != message_a, "content equality must not deduplicate ordinary writes"

    operation_key = f"http-operation-{uuid.uuid4()}"
    keyed_payload = {
        "session_id": session, "role": "assistant", "content": "keyed-content",
        "operation_key": operation_key, **scope_a,
    }
    keyed = call("add_message", keyed_payload)
    replay = call("add_message", keyed_payload)
    assert replay["id"] == keyed["id"], "same operation key and payload must replay"
    call("add_message", {
        **keyed_payload, "content": "conflicting-content",
    }, 400)

    for scope, _, _ in resources:
        call("clear_session", {"session_id": session, **scope}, 204)
    print(json.dumps({"status": "PASS", "scope_count": 3, "http": True,
                      "operation_key": "replay-and-conflict-pass",
                      "admin_reset": False, "encoder": "synthetic fixture only"}))


if __name__ == "__main__":
    main()
