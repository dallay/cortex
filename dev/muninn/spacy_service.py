"""Loopback-only spaCy stage. Loads a local installed model once; never downloads.

Run with --model pointing to the locally installed en_core_web_sm-3.8.0 model
folder. See docs/muninn/extraction.md for offline installation and Rust wiring.
"""
import argparse
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import json
from pathlib import Path
import threading

MAX_BODY = 1_048_576
# Every accepted connection gets a finite read deadline. Without it a client that
# promises a body and never sends it holds the socket open forever.
READ_TIMEOUT_SECONDS = 10.0
TYPE_MAPPING = {
    "PERSON": "PERSON", "ORG": "ORGANIZATION", "NORP": "ORGANIZATION",
    "GPE": "LOCATION", "LOC": "LOCATION", "FAC": "LOCATION", "EVENT": "EVENT",
    "PRODUCT": "OBJECT", "WORK_OF_ART": "OBJECT", "LAW": "OBJECT", "LANGUAGE": "OBJECT",
    "DATE": "EVENT", "TIME": "EVENT", "MONEY": "OBJECT", "QUANTITY": "OBJECT",
    "ORDINAL": "OBJECT", "CARDINAL": "OBJECT", "PERCENT": "OBJECT",
}
SUBTYPE_MAPPING = {
    "GPE": "GEOPOLITICAL", "LOC": "GEOGRAPHIC", "FAC": "FACILITY",
    "PRODUCT": "PRODUCT", "WORK_OF_ART": "CREATIVE_WORK", "LAW": "LEGAL_DOCUMENT",
    "LANGUAGE": "LANGUAGE", "DATE": "DATE", "TIME": "TIME", "MONEY": "CURRENCY",
    "QUANTITY": "QUANTITY", "NORP": "GROUP",
}


def extract(payload, nlp, *, confidence=0.85):
    """Pinned spaCy mapping, character offsets and constant (not model) confidence."""
    if not isinstance(payload, dict) or not isinstance(payload.get("text"), str):
        raise ValueError("text must be a string")
    types = payload.get("entity_types")
    if types is not None and (not isinstance(types, list)
                              or any(not isinstance(item, str) for item in types)):
        raise ValueError("entity_types must be a list of strings")
    text = payload["text"]
    entities = []
    if text.strip() and confidence >= 0.85:
        for span in nlp(text).ents:
            kind = TYPE_MAPPING.get(span.label_, "OBJECT")
            if types and kind not in types:
                continue
            entities.append({
                "name": span.text.strip(), "type": kind,
                "subtype": SUBTYPE_MAPPING.get(span.label_),
                "start_pos": span.start_char, "end_pos": span.end_char,
                "confidence": confidence, "extractor": "spacy",
            })
    return {"entities": entities, "relations": [], "preferences": [], "source_text": text}


def handler(nlp):
    class Handler(BaseHTTPRequestHandler):
        # StreamRequestHandler.setup applies this to the accepted socket, so it
        # bounds the request line, the headers and the body read alike.
        timeout = READ_TIMEOUT_SECONDS

        def log_message(self, *_):
            pass  # Never log request text, paths or headers.

        def do_POST(self):
            if self.path != "/extract":
                self.reply(404, {"error": "unknown endpoint"})
                return
            try:
                length = int(self.headers.get("Content-Length", "0"))
                if not 0 < length <= MAX_BODY:
                    raise ValueError("invalid body length")
                payload = json.loads(self.rfile.read(length))
                result = extract(payload, nlp)
            except (TimeoutError, OSError):
                self.reply(408, {"error": "request body not received in time"})
                return
            except (ValueError, UnicodeError):
                self.reply(400, {"error": "invalid extraction request"})
                return
            except Exception:
                self.reply(500, {"error": "spaCy extraction failed"})
                return
            self.reply(200, result)

        def reply(self, status, payload):
            data = json.dumps(payload).encode("utf-8")
            # One request per connection: no keep-alive socket is left holding. 
            self.close_connection = True
            try:
                self.send_response(status)
                self.send_header("Content-Type", "application/json")
                self.send_header("Content-Length", str(len(data)))
                self.end_headers()
                self.wfile.write(data)
            except OSError:
                pass  # Peer already gone; nothing left to report.
    return Handler


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--model", type=Path, required=True,
                        help="Local en_core_web_sm-3.8.0 model directory (no download)")
    parser.add_argument("--port", type=int, default=8765)
    args = parser.parse_args()
    if not args.model.is_dir():
        parser.error("model must be an existing local directory")
    import spacy
    pipeline = spacy.load(args.model)
    # A spaCy Language is not documented as thread-safe, so the pipeline is
    # serialized while the socket layer stays concurrent.
    lock = threading.Lock()

    def nlp(text):
        with lock:
            return pipeline(text)

    # Threaded so one slow client cannot serialize every extraction request.
    server = ThreadingHTTPServer(("127.0.0.1", args.port), handler(nlp))
    server.daemon_threads = True
    server.serve_forever()


if __name__ == "__main__":
    main()
