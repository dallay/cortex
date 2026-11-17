"""Regresiones de inventario estático sin importar upstream."""

import importlib.util
from pathlib import Path
import sys
import unittest


def load_inventory():
    path = Path(__file__).resolve().parents[1] / "api_inventory.py"
    if not path.is_file():
        raise AssertionError("Falta el inventario canónico")
    spec = importlib.util.spec_from_file_location("api_inventory", path)
    module = importlib.util.module_from_spec(spec)
    sys.path.insert(0, str(path.parent))
    try:
        spec.loader.exec_module(module)
    finally:
        sys.path.pop(0)
    return module


class DeclarationTests(unittest.TestCase):
    def test_backend_binding_does_not_inherit_bolt_methods_into_nams(self):
        inventory = load_inventory()
        files = {
            "client.py": "class Client:\n    @property\n    def memory(self): return self._memory\n",
            "protocol.py": "class Contract:\n    async def required(self): ...\n",
            "base.py": "class Base:\n    async def inherited(self): return 1\n",
            "bolt.py": "class Bolt(Base):\n    async def required(self): return 2\n",
            "nams.py": "class Nams:\n    async def required(self): raise NotSupportedError('nams')\n",
        }
        bindings = {"facade": {"path": "client.py", "class": "Client"}, "accessors": {
            "memory": {"contract": {"path": "protocol.py", "class": "Contract"},
                       "bolt": {"path": "bolt.py", "class": "Bolt", "base": {"path": "base.py", "class": "Base"}},
                       "nams": {"path": "nams.py", "class": "Nams"}},
        }}
        rows = inventory.profile_rows(files.__getitem__, bindings, {}, "a" * 40)
        by_key = {(r["operation"], r["backend"]): r for r in rows}
        self.assertEqual(by_key[("client.memory.required", "nams")]["static_state"], "rechazo_explicito")
        self.assertEqual(by_key[("client.memory.inherited", "nams")]["static_state"], "sin_binding")
        self.assertEqual(by_key[("client.memory.inherited", "nams")]["kind"], "no_declarado")
        self.assertIn("base.py", by_key[("client.memory.inherited", "bolt")]["implementation_source"])
        self.assertTrue(all(r["muninn_status"] == "no_implementado" and r["runtime_status"] == "no_verificado" for r in rows))
        files["client.py"] += "\ndef connect(backend: str = 'bolt'): return Client()\n"
        bindings["factory"] = {"path": "client.py", "name": "connect"}
        rows = inventory.profile_rows(files.__getitem__, bindings, {}, "a" * 40)
        self.assertEqual({r["backend"] for r in rows if r["operation"] == "connect"}, {"bolt", "nams"})
        with self.assertRaisesRegex(ValueError, "nota"):
            inventory.profile_rows(files.__getitem__, bindings, {"nams:client.missing": {"note": "typo"}}, "a" * 40)
        files["bolt.py"] = files["bolt.py"].replace("Bolt(Base)", "Bolt")
        with self.assertRaisesRegex(ValueError, "herencia"):
            inventory.profile_rows(files.__getitem__, bindings, {}, "a" * 40)

    def test_sentinel_lookup_and_exports_do_not_become_supported_methods(self):
        inventory = load_inventory()
        files = {
            "client.py": "class Client:\n    @property\n    def auth(self): return self._auth\n",
            "auth.py": "class Auth:\n    def rotate(self): return True\n",
            "unsupported.py": "class Sentinel:\n    def __getattr__(self,name): raise NotSupportedError()\n",
        }
        bindings = {"facade": {"path": "client.py", "class": "Client"},
                    "unsupported": {"path": "unsupported.py", "class": "Sentinel"},
                    "accessors": {"auth": {"bolt": {"sentinel": True},
                                            "nams": {"path": "auth.py", "class": "Auth",
                                                     "conditional": True, "method_note": "Solo REST"}}}}
        rows = inventory.profile_rows(files.__getitem__, bindings, {}, "a" * 40)
        by_key = {(r["operation"],r["backend"]):r for r in rows}
        self.assertEqual(by_key[("client.auth.rotate","bolt")]["static_state"], "rechazo_resolver_atributo")
        self.assertIn("unsupported.py", by_key[("client.auth.rotate","bolt")]["implementation_source"])
        self.assertEqual(by_key[("client.auth.rotate","nams")]["static_state"], "condicional")
        self.assertIn("Solo REST", by_key[("client.auth.rotate","nams")]["notes"])
        exports=inventory.exported_names("raise RuntimeError()\n__all__ = ['Client','Auth']\n", "a"*40, "client.py")
        self.assertEqual([e["name"] for e in exports], ["Client","Auth"])
        with self.assertRaises(ValueError):
            inventory.exported_names("__all__ = load_plugins()", "a"*40, "client.py")

    def test_contract_rejection_and_empty_body_do_not_become_verified_capabilities(self):
        inventory = load_inventory()
        source = '''
raise RuntimeError("no importar")
class API:
    async def contract(self, limit: int = 10): ...
    async def unsupported(self):
        raise NotSupportedError("backend")
    async def empty(self): return ""
    @property
    def pending(self): return 0
    def _private(self): pass
'''
        methods = inventory.class_members(source, "API")
        self.assertEqual(set(methods), {"contract", "unsupported", "empty", "pending"})
        self.assertEqual(methods["contract"]["state"], "declaracion_contractual")
        self.assertEqual(methods["unsupported"]["state"], "rechazo_explicito")
        self.assertEqual(methods["empty"]["state"], "retorno_constante_vacio")
        self.assertEqual(methods["pending"]["kind"], "property")
        self.assertIn("limit: int=10", methods["contract"]["signature"])
        self.assertEqual(methods["contract"]["line"], 4)
        with self.assertRaises(ValueError):
            inventory.class_members(source, "Missing")


if __name__ == "__main__":
    unittest.main()
