"""Un PASS de transporte no sustituye persistencia real."""

import importlib.util
from pathlib import Path
import unittest


class PersistenceTests(unittest.TestCase):
    def test_requires_message_and_conversation_ids_from_actual_graph(self):
        path=Path(__file__).resolve().parents[1]/"rust_spike_probe.py"
        self.assertTrue(path.is_file(),"Falta la verificación directa Neo4j")
        spec=importlib.util.spec_from_file_location("rust_spike_probe",path)
        module=importlib.util.module_from_spec(spec);spec.loader.exec_module(module)
        expected={"session_id":"s","conversation_id":"c","message_id":"m","content":"First message"}
        graph={"owner_valid":True,"conversations":1,"messages":1,"has_message":1,"first_message":1,
               "rows":[{"session_id":"s","conversation_id":"c","message_id":"m","content":"First message","role":"user","metadata":"{}"}]}
        self.assertTrue(module.valid_persistence(expected,graph))
        graph["rows"][0]["message_id"]="fake"
        self.assertFalse(module.valid_persistence(expected,graph))
        graph["rows"]=[]
        self.assertFalse(module.valid_persistence(expected,graph))


if __name__=="__main__":unittest.main()
