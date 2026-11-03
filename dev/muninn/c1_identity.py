"""Offline C1 declaration, not execution authentication or policy approval."""
import hashlib
import json
import re
from pathlib import Path
from minilm_cpu_oracle import load_pinned_lock

BUNDLE_ID = 'ee963db150ae720729aebee53794830f1d0e60960c9253e575a049b799758ac1'
LOCK = Path(__file__).resolve().parents[2] / 'dev/muninn/fixtures/artifact-lock.json'

def declare(numeric_contract_version, lock=None):
    if type(numeric_contract_version) is not str or re.fullmatch(r'[a-z0-9][a-z0-9._-]{0,63}', numeric_contract_version) is None:
        raise ValueError('numeric_contract_version')
    pinned = load_pinned_lock(LOCK.read_bytes())
    if lock is not None and lock != pinned:
        raise ValueError('manifest')
    model = next(m for m in pinned['models'] if m['id'] == 'sentence-transformers/all-MiniLM-L6-v2')
    files = sorted(({k: f[k] for k in ('path', 'size', 'sha256')} for f in model['files']), key=lambda f: f['path'])
    receipt = {'schema_version': 1, 'model_id': model['id'],
               'revision': model['revision'], 'files': files}
    bundle = hashlib.sha256(json.dumps(receipt, separators=(',', ':')).encode()).hexdigest()
    if bundle != BUNDLE_ID:
        raise ValueError('bundle')
    compatibility = {'provider_id': 'local-cpu', 'model_id': model['id'],
                     'model_revision': model['revision'],
                     'model_files': [f for f in files if f['path'] != 'README.md'],
                     'dimensions': 384, 'normalization': 'l2', 'similarity_function': 'cosine',
                     'numeric_contract_version': numeric_contract_version, 'embedding_schema_version': 1}
    canonical = json.dumps(['muninn-embedding-space-v1', compatibility['provider_id'],
                            compatibility['model_id'], compatibility['model_revision'],
                            [[f['path'], f['size'], f['sha256']] for f in compatibility['model_files']],
                            384, 'l2', 'cosine', numeric_contract_version, 1],
                           separators=(',', ':'), ensure_ascii=False).encode()
    return {'schema_version': 1, 'scope': 'identity_declaration_not_execution_authentication',
            'artifact_bundle_id': bundle, 'space_id': hashlib.sha256(canonical).hexdigest(),
            'compatibility': compatibility, 'provisioned_indices': []}

def validate(value, numeric_contract_version):
    expected = declare(numeric_contract_version)
    # JSON equality must not admit bool-as-int or float-as-int compatibility values.
    if json.dumps(value, sort_keys=True, separators=(',', ':')) != json.dumps(expected, sort_keys=True, separators=(',', ':')):
        raise ValueError('identity')
