# /// script
# requires-python = ">=3.10"
# dependencies = ["openai==2.11.0"]
# ///
"""Validate real Rook HTTP fixtures against openai-python==2.11.0.

Run from the repository root: uv run scripts/verify_responses_sdk.py
No real credentials or external services are required.
"""

import json
import os
from pathlib import Path
import subprocess
import tempfile

import httpx
from openai import APIError, OpenAI
from openai.types.responses import Response, ResponseStreamEvent
from pydantic import TypeAdapter

from responses_sse import parse_sse_frames


def wire_value(value):
    """Remove SDK-local parsing fields that are not part of the wire protocol."""
    if isinstance(value, dict):
        return {key: wire_value(item) for key, item in value.items()
                if key not in {"parsed", "parsed_arguments"}}
    if isinstance(value, list):
        return [wire_value(item) for item in value]
    return value


def main() -> None:
    root = Path(__file__).resolve().parents[1]
    with tempfile.TemporaryDirectory(prefix="rook-responses-") as directory:
        subprocess.run(
            ["cargo", "test", "-p", "transport-axum", "--test", "gateway_e2e", "responses"],
            cwd=root,
            env={**os.environ, "ROOK_RESPONSES_FIXTURES_DIR": directory},
            check=True,
        )
        fixtures = Path(directory)
        required = {"text.json", "openai-tool.json", "anthropic-tool.json",
                    "openai-tool.sse", "anthropic-tool.sse", "text-stop.sse",
                    "text-length.sse", "text-content_filter.sse", "failed-error.sse",
                    "failed-done.sse", "failed-eof.sse", "incomplete.json",
                    "partial-tool-length-done.sse", "partial-tool-length-eof.sse",
                     "partial-tool-content_filter-done.sse", "partial-tool-content_filter-eof.sse",
                     "empty-length-standard-done.sse", "empty-length-standard-eof.sse",
                     "empty-length-combo-done.sse", "empty-length-combo-eof.sse",
                     "empty-content_filter-standard-done.sse", "empty-content_filter-standard-eof.sse",
                     "empty-content_filter-combo-done.sse", "empty-content_filter-combo-eof.sse",
                     "validation-http-error-standard.json",
                     "validation-http-error-combo.json",
                     "validation-first-standard.sse", "validation-first-combo-fallback.sse",
                     "validation-late-false-false.sse", "validation-late-false-true.sse",
                     "validation-late-true-false.sse", "validation-late-true-true.sse",
                     "anthropic-stop-functions-false.sse", "anthropic-stop-functions-true.sse",
                     "anthropic-initial-empty-false.sse", "anthropic-initial-empty-true.sse",
                     "anthropic-initial-object-false.sse", "anthropic-initial-object-true.sse",
                     "anthropic-missing-finish-false-false.sse", "anthropic-missing-finish-false-true.sse",
                     "anthropic-missing-finish-true-false.sse", "anthropic-missing-finish-true-true.sse",
                     "anthropic-malformed-false-false.sse", "anthropic-malformed-false-true.sse",
                     "anthropic-malformed-true-false.sse", "anthropic-malformed-true-true.sse",
                     "anthropic-truncated-false-false.sse", "anthropic-truncated-false-true.sse",
                     "anthropic-truncated-true-false.sse", "anthropic-truncated-true-true.sse",
                     "anthropic-generated-id-false.sse", "anthropic-generated-id-true.sse",
                     "empty-name.sse", "interleaved-replay.sse",
                     "terminal-nonobject-false-false.sse", "terminal-nonobject-false-true.sse",
                     "terminal-nonobject-true-false.sse", "terminal-nonobject-true-true.sse",
                     "validation-http-error-standard.json",
                     "validation-http-error-combo.json"}
        fixture_paths = sorted(fixtures.iterdir())
        fixture_names = {path.name for path in fixture_paths}
        assert required <= fixture_names
        regression_fixtures = {
            f"anthropic-initial-failure-{error}-{text}-{combo}.sse"
            for error in ("false", "true") for text in ("false", "true")
            for combo in ("false", "true")
        } | {
            f"anthropic-late-usage-{upstream_id}-{null}-{combo}.sse"
            for upstream_id in ("false", "true") for null in ("false", "true")
            for combo in ("false", "true")
        } | {
            f"anthropic-unknown-after-finish-{tool}-{combo}.sse"
            for tool in ("false", "true") for combo in ("false", "true")
        } | {
            f"anthropic-updated-usage-{zero}-{null}-{combo}.sse"
            for zero in ("false", "true") for null in ("false", "true")
            for combo in ("false", "true")
        } | {
            f"openai-finish-usage-{prior}-{reason}-{combo}.sse"
            for prior in ("false", "true") for reason in ("unknown", "absent", "null")
            for combo in ("false", "true")
        } | {
            f"openai-initial-unknown-{combo}.sse" for combo in ("false", "true")
        } | {
            f"anthropic-nonstream-generated-ids-{combo}.json" for combo in ("false", "true")
        } | {
            f"anthropic-null-id-{combo}.sse" for combo in ("false", "true")
        } | {
            f"anthropic-null-id-{combo}-http-error.json" for combo in ("false", "true")
        } | {
            f"openai-generated-ids-{combo}.{extension}"
            for combo in ("false", "true") for extension in ("json", "sse")
        } | {
            f"anthropic-invalid-id-{kind}-{combo}{suffix}"
            for kind in ("number", "negative", "float", "boolean", "array", "object")
            for combo in ("false", "true") for suffix in (".sse", "-http-error.json")
        } | {
            f"openai-invalid-id-{kind}-{combo}{suffix}"
            for kind in ("null", "empty", "control", "long", "number", "boolean", "array", "object")
            for combo in ("false", "true") for suffix in (".sse", "-http-error.json")
            if kind != "null" or suffix != ".sse"
        }
        assert regression_fixtures <= fixture_names
        adapter = TypeAdapter(ResponseStreamEvent)
        count = 0
        for path in fixture_paths:
            body = path.read_text()
            if path.name.endswith("-http-error.json") or path.stem.startswith("validation-http-error-"):
                transport = httpx.MockTransport(lambda _: httpx.Response(502, json=json.loads(body)))
                with OpenAI(api_key="test", base_url="http://rook.invalid/v1",
                            max_retries=0, http_client=httpx.Client(transport=transport)) as client:
                    try:
                        client.responses.create(model="gpt-4o", input="hola", store=False)
                    except APIError as error:
                        assert error.status_code == 502
                        assert error.code == "upstream_error"
                    else:
                        raise AssertionError("HTTP 502 must raise APIError")
            elif path.suffix == ".json":
                expected = Response.model_validate_json(body, strict=True)
                transport = httpx.MockTransport(lambda _: httpx.Response(200, json=json.loads(body)))
                with OpenAI(api_key="test", base_url="http://rook.invalid/v1",
                            http_client=httpx.Client(transport=transport)) as client:
                    actual = client.responses.create(model=expected.model, input="hola", store=False)
                    assert actual.id == expected.id
                    assert actual.output == expected.output
                    assert actual.output_text == expected.output_text
            else:
                frames = parse_sse_frames(body)
                events = [event for _, event in frames]
                for index, event in enumerate(events):
                    adapter.validate_python(event, strict=True)
                    assert event["sequence_number"] == index
                terminals = [e for e in events if e["type"] in {
                    "response.completed", "response.incomplete", "response.failed"}]
                assert len(terminals) == 1
                Response.model_validate(terminals[0]["response"], strict=True)
                if terminals[0]["type"] != "response.failed":
                    transport = httpx.MockTransport(lambda _: httpx.Response(
                        200, headers={"content-type": "text/event-stream"}, content=body))
                    with OpenAI(api_key="test", base_url="http://rook.invalid/v1",
                                http_client=httpx.Client(transport=transport)) as client:
                        expected = Response.model_validate(terminals[0]["response"], strict=True)
                        if terminals[0]["type"] == "response.completed":
                            with client.responses.stream(model="gpt-4o", input="hola", store=False) as stream:
                                list(stream)
                                actual = stream.get_final_response()
                        else:
                            # In 2.11.0 get_final_response retains only response.completed.
                            with client.responses.create(model="gpt-4o", input="hola", store=False, stream=True) as stream:
                                actual = list(stream)[-1].response
                        assert actual.id == expected.id
                        assert wire_value([i.model_dump() for i in actual.output]) == [i.model_dump() for i in expected.output]
                else:
                    # In 2.11.0 Responses SSE errors are typed events, not APIError:
                    # the reader raises only when data contains an error field.
                    for accumulate in [False, True]:
                        transport = httpx.MockTransport(lambda _: httpx.Response(
                            200, headers={"content-type": "text/event-stream"}, content=body))
                        with OpenAI(api_key="test", base_url="http://rook.invalid/v1",
                                    http_client=httpx.Client(transport=transport)) as client:
                            observed = []
                            manager = (client.responses.stream(model="gpt-4o", input="hola", store=False)
                                       if accumulate else client.responses.create(
                                           model="gpt-4o", input="hola", store=False, stream=True))
                            with manager as stream:
                                for item in stream:
                                    observed.append(item.type)
                                if accumulate:
                                    try:
                                        stream.get_final_response()
                                    except RuntimeError as error:
                                        assert "response.completed" in str(error)
                                    else:
                                        raise AssertionError("A failed stream must not yield a successful final response")
                            assert "response.completed" not in observed
                            assert "error" in observed
                            assert observed[-1] == "response.failed"
                count += len(events)
            print(f"PASS {path.name}")
        print(f"PASS openai-python==2.11.0: {len(fixture_paths)} fixtures; {count} events")


if __name__ == "__main__":
    main()
