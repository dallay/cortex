"""Deterministic stage tests: no spaCy installation or model downloads."""
import http.client
import importlib.util
import io
import math
from pathlib import Path
import socket
import threading
import time
from types import SimpleNamespace
import unittest


def stage():
    path = Path(__file__).resolve().parents[1] / "spacy_service.py"
    spec = importlib.util.spec_from_file_location("spacy_service", path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


class SpacyServiceTests(unittest.TestCase):
    def test_confidence_boundary(self):
        service = stage()
        span = SimpleNamespace(text=" Alice ", label_="PERSON", start_char=0, end_char=7)
        nlp = lambda _: SimpleNamespace(ents=[span])
        for confidence, count in [(0.849999, 0), (0.85, 1), (0.850001, 1)]:
            result = service.extract({"text": " Alice "}, nlp, confidence=confidence)
            self.assertEqual(len(result["entities"]), count)
            if count:
                self.assertEqual(result["entities"][0], {
                    "name": "Alice", "type": "PERSON", "subtype": None,
                    "start_pos": 0, "end_pos": 7, "confidence": confidence,
                    "extractor": "spacy",
                })
            self.assertEqual(result["relations"], [])
            self.assertEqual(result["preferences"], [])

    def test_all_pinned_labels_and_subtypes(self):
        service = stage()
        expected = {
            "PERSON": ("PERSON", None), "ORG": ("ORGANIZATION", None),
            "NORP": ("ORGANIZATION", "GROUP"), "GPE": ("LOCATION", "GEOPOLITICAL"),
            "LOC": ("LOCATION", "GEOGRAPHIC"), "FAC": ("LOCATION", "FACILITY"),
            "EVENT": ("EVENT", None), "PRODUCT": ("OBJECT", "PRODUCT"),
            "WORK_OF_ART": ("OBJECT", "CREATIVE_WORK"), "LAW": ("OBJECT", "LEGAL_DOCUMENT"),
            "LANGUAGE": ("OBJECT", "LANGUAGE"), "DATE": ("EVENT", "DATE"),
            "TIME": ("EVENT", "TIME"), "MONEY": ("OBJECT", "CURRENCY"),
            "QUANTITY": ("OBJECT", "QUANTITY"), "ORDINAL": ("OBJECT", None),
            "CARDINAL": ("OBJECT", None), "PERCENT": ("OBJECT", None),
            "UNKNOWN": ("OBJECT", None),
        }
        for label, (kind, subtype) in expected.items():
            nlp = lambda _, label=label: SimpleNamespace(ents=[SimpleNamespace(
                text="x", label_=label, start_char=0, end_char=1)])
            entity = service.extract({"text": "x"}, nlp)["entities"][0]
            self.assertEqual((entity["type"], entity["subtype"]), (kind, subtype))
            self.assertEqual(service.extract({"text": "x", "entity_types": ["PERSON"]}, nlp)
                             ["entities"], [entity] if kind == "PERSON" else [])

    def test_http_errors_are_static_without_network_or_model(self):
        service = stage()
        def forbidden(_):
            self.fail("invalid HTTP request reached model")
        for path, length, body, status, error in [
            ("/unknown", "2", b"{}", 404, "unknown endpoint"),
            ("/extract", "0", b"", 400, "invalid extraction request"),
            ("/extract", "1048577", b"", 400, "invalid extraction request"),
            ("/extract", "-1", b"", 400, "invalid extraction request"),
            ("/extract", "3", b"bad", 400, "invalid extraction request"),
            ("/extract", "2", b"{}", 400, "invalid extraction request"),
        ]:
            handler = service.handler(forbidden)
            request = handler.__new__(handler)
            request.path = path
            request.headers = {"Content-Length": length}
            request.rfile = io.BytesIO(body)
            replies = []
            request.reply = lambda code, payload: replies.append((code, payload))
            request.do_POST()
            self.assertEqual(replies, [(status, {"error": error})])
        def failed(_):
            raise RuntimeError("private synthetic model error")
        handler = service.handler(failed)
        request = handler.__new__(handler)
        request.path = "/extract"
        body = b'{"text":"Alice"}'
        request.headers = {"Content-Length": str(len(body))}
        request.rfile = io.BytesIO(body)
        replies = []
        request.reply = lambda code, payload: replies.append((code, payload))
        request.do_POST()
        self.assertEqual(replies, [(500, {"error": "spaCy extraction failed"})])

    def test_malformed_input_never_calls_model(self):
        service = stage()
        def forbidden(_):
            self.fail("invalid input reached model")
        for payload in [None, [], {}, {"text": 1}, {"text": "x", "entity_types": "PERSON"},
                        {"text": "x", "entity_types": [1]}]:
            with self.assertRaises(ValueError):
                service.extract(payload, forbidden)
        self.assertEqual(service.extract({"text": "  "}, forbidden)["entities"], [])

    def serve(self, service, nlp, timeout, handler=None):
        previous = service.READ_TIMEOUT_SECONDS
        service.READ_TIMEOUT_SECONDS = timeout
        self.addCleanup(setattr, service, "READ_TIMEOUT_SECONDS", previous)
        factory = handler or service.handler
        server = service.ThreadingHTTPServer(("127.0.0.1", 0), factory(nlp))
        server.daemon_threads = True
        worker = threading.Thread(target=server.serve_forever, daemon=True)
        worker.start()
        self.addCleanup(worker.join, 2)
        self.addCleanup(server.server_close)
        self.addCleanup(server.shutdown)
        return server.server_address

    def test_accepted_connections_have_a_finite_deadline(self):
        service = stage()
        observed = []
        def recording(nlp):
            class Observed(service.handler(nlp)):
                def setup(self):
                    super().setup()
                    observed.append(self.connection.gettimeout())
            return Observed
        address = self.serve(service, lambda _: SimpleNamespace(ents=[]), 0.3,
                             handler=recording)
        connection = http.client.HTTPConnection(*address, timeout=2)
        try:
            connection.request("POST", "/extract", b'{"text":"Alice"}',
                               {"Content-Type": "application/json"})
            self.assertEqual(connection.getresponse().status, 200)
        finally:
            connection.close()
        self.assertEqual(len(observed), 1)
        self.assertIsNotNone(observed[0])
        self.assertTrue(math.isfinite(observed[0]))
        self.assertGreater(observed[0], 0)

    def test_withheld_body_cannot_block_other_requests(self):
        service = stage()
        address = self.serve(service, lambda _: SimpleNamespace(ents=[]), 0.5)
        stalled = socket.create_connection(address, timeout=2)
        self.addCleanup(stalled.close)
        stalled.sendall(b"POST /extract HTTP/1.1\r\nHost: stage\r\n"
                        b"Content-Length: 16\r\n\r\n")
        connection = http.client.HTTPConnection(*address, timeout=2)
        try:
            connection.request("POST", "/extract", b'{"text":"Alice"}',
                               {"Content-Type": "application/json"})
            self.assertEqual(connection.getresponse().status, 200)
        finally:
            connection.close()
        # The stalled request times out on its own, so the socket closes.
        deadline = socket.timeout
        stalled.settimeout(3)
        try:
            self.assertEqual(stalled.recv(4096)[:12], b"HTTP/1.0 408"[:12])
        except deadline:
            self.fail("stalled request was never timed out")
        finally:
            stalled.close()

    def test_idle_connection_is_closed_without_serving(self):
        """A client that sends no request line is closed, never held open."""
        service = stage()
        address = self.serve(service, lambda _: SimpleNamespace(ents=[]), 0.3)
        idle = socket.create_connection(address, timeout=2)
        self.addCleanup(idle.close)
        idle.settimeout(3)
        self.assertEqual(idle.recv(4096), b"")

    def test_stalled_body_does_not_delay_a_concurrent_request(self):
        """The shipped deadline must not serialize other extraction requests."""
        service = stage()
        address = self.serve(service, lambda _: SimpleNamespace(ents=[]), 2.0)
        stalled = socket.create_connection(address, timeout=2)
        self.addCleanup(stalled.close)
        stalled.sendall(b"POST /extract HTTP/1.1\r\nHost: stage\r\n"
                        b"Content-Length: 16\r\n\r\n")
        start = time.monotonic()
        connection = http.client.HTTPConnection(*address, timeout=2)
        try:
            connection.request("POST", "/extract", b'{"text":"Alice"}',
                               {"Content-Type": "application/json"})
            self.assertEqual(connection.getresponse().status, 200)
        finally:
            connection.close()
        self.assertLess(time.monotonic() - start, 1.0)

    def test_body_timeout_replies_static_408_without_echo(self):
        """The timeout branch returns a fixed reply and never reaches the model."""
        service = stage()

        class Stalled(io.BytesIO):
            def read(self, *_):
                raise TimeoutError("simulated read deadline")

        handler = service.handler(lambda _: self.fail("timed-out body reached model"))
        request = handler.__new__(handler)
        request.path = "/extract"
        request.headers = {"Content-Length": "22"}
        request.rfile = Stalled()
        replies = []
        request.reply = lambda code, payload: replies.append((code, payload))
        request.do_POST()
        self.assertEqual(replies, [(408, {"error": "request body not received in time"})])
