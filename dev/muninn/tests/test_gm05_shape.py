"""La forma esperada se deriva de las fuentes reales, no de fixtures idealizadas.

GM05-005 y GM05-012:

- `crates/muninn/bridge/src/wire.rs` define la forma que emite el bridge; la TCK
  normativa exige esos nombres, así que el contrato debe traducirlos, no cambiarlos.
- `long_term.py` y `core/memory.py` definen el modelo que el oráculo puede
  devolver; el contrato debe conocer todos esos campos.
- Un campo desconocido bloquea la comparación, de modo que el contrato tiene que
  declarar la superficie completa: comparable o no comparable con justificación.
- El contrato no puede exigir un campo comparable que el wire nunca emite.
- Los atributos del escenario se leen de `gm05_capture`; el contrato no los
  escribe a mano.
"""

import ast
import importlib.util
import re
from pathlib import Path
import sys
import unittest


BASE = Path(__file__).resolve().parents[1]
ROOT = BASE.parents[1]
sys.path.insert(0, str(BASE))

WIRE = ROOT / 'crates/muninn/bridge/src/wire.rs'
ORACLE = BASE / '.cache/sources/agent-memory/src/neo4j_agent_memory'
WIRE_FUNCTIONS = {'entity': 'entity', 'preference': 'preference',
                  'fact': 'fact', 'relationship': 'relationship'}
MODELS = {'Entity': 'entity', 'Preference': 'preference',
          'Fact': 'fact', 'Relationship': 'relationship'}


def module(name):
    spec = importlib.util.spec_from_file_location(name + "_under_test", BASE / (name + ".py"))
    loaded = importlib.util.module_from_spec(spec)
    sys.modules[spec.name] = loaded
    spec.loader.exec_module(loaded)
    return loaded


CONTRACT = module('gm05_contract')
ADAPTERS = module('gm05_adapters')
CAPTURE = module('gm05_capture')


def wire_surface(category):
    """Campos que el bridge emite para una categoría, en nombres del contrato."""
    text = WIRE.read_text(encoding='utf-8')
    start = text.index(f'pub fn {WIRE_FUNCTIONS[category]}(')
    body = text[start:text.index('\n}\n', start)]
    emitted = re.findall(r'"([a-z_]+)"\s*:', body[body.index('json!('):])
    aliases = ADAPTERS.WIRE_NAMES.get(category, {})
    return {aliases.get(name, name) for name in emitted}


def model_surface(path, classes):
    tree = ast.parse(path.read_text(encoding='utf-8'))
    surfaces = {}
    for node in tree.body:
        if isinstance(node, ast.ClassDef) and node.name in classes:
            surfaces[node.name] = {statement.target.id for statement in node.body
                                   if isinstance(statement, ast.AnnAssign)
                                   and isinstance(statement.target, ast.Name)}
    return surfaces


class WireShapeTests(unittest.TestCase):
    """El contrato conoce la superficie que el producto emite hoy."""

    def test_the_wire_surface_is_declared_by_the_contract(self):
        for category in WIRE_FUNCTIONS:
            with self.subTest(category=category):
                self.assertEqual(sorted(wire_surface(category)
                                        - CONTRACT.KNOWN_FIELDS[category]), [])

    def test_the_contract_does_not_demand_fields_the_wire_never_emits(self):
        for category, comparable in CONTRACT.COMPARABLE_FIELDS.items():
            with self.subTest(category=category):
                self.assertEqual(sorted(comparable - wire_surface(category)), [])

    def test_the_wire_translation_only_names_contract_fields(self):
        for category, aliases in ADAPTERS.WIRE_NAMES.items():
            with self.subTest(category=category):
                self.assertEqual(sorted(set(aliases.values())
                                        - CONTRACT.KNOWN_FIELDS[category]), [])

    def test_the_wire_relationship_does_not_emit_generated_marks(self):
        emitted = wire_surface('relationship')
        self.assertEqual(sorted(emitted & {'created_at', 'updated_at'}), [])


@unittest.skipUnless(ORACLE.is_dir(), 'requiere las fuentes fijadas del oráculo')
class OracleModelShapeTests(unittest.TestCase):
    """El contrato conoce el modelo que el oráculo puede devolver."""

    def base(self):
        return model_surface(ORACLE / 'core/memory.py', {'MemoryEntry'})['MemoryEntry']

    def models(self):
        return model_surface(ORACLE / 'memory/long_term.py', set(MODELS))

    def test_the_oracle_model_surface_is_declared_by_the_contract(self):
        base = self.base()
        models = self.models()
        for name, category in MODELS.items():
            with self.subTest(model=name):
                fields = base | models[name]
                self.assertEqual(sorted(fields - CONTRACT.KNOWN_FIELDS[category]), [])

    def test_the_oracle_model_keeps_the_scenario_fields_comparable(self):
        base = self.base()
        models = self.models()
        for name, category in MODELS.items():
            with self.subTest(model=name):
                comparable = CONTRACT.COMPARABLE_FIELDS[category]
                self.assertEqual(sorted(comparable - (base | models[name])), [])


class ScenarioTests(unittest.TestCase):
    """Los atributos esperados se leen del escenario, no se escriben a mano."""

    def test_the_contract_does_not_hardcode_the_scenario_attributes(self):
        attributes = CAPTURE.RELATIONSHIP['attributes']
        self.assertTrue(attributes, 'el escenario debe fijar atributos')
        source = (BASE / 'gm05_contract.py').read_text(encoding='utf-8')
        for key, value in attributes.items():
            with self.subTest(attribute=key):
                self.assertNotIn(f"'{key}'", source)
                self.assertNotIn(f"'{value}'", source)

    def test_the_deviation_reads_the_scenario_attributes(self):
        attributes = CAPTURE.RELATIONSHIP['attributes']
        self.assertEqual(CONTRACT.RELATIONSHIP['attributes'], attributes)
        source = (BASE / 'gm05_contract.py').read_text(encoding='utf-8')
        self.assertIn('from gm05_capture import RELATIONSHIP', source)


if __name__ == '__main__':
    unittest.main()
