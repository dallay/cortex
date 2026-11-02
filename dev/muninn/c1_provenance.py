"""Authenticate both C1 build subjects with GitHub's Sigstore verifier.

A build record is untrusted until gh verifies it AND the executable with the
same protected-main source identity. No JSON verifier output is trusted.
"""
import hashlib
import json
from pathlib import Path
import re
import subprocess

REPOSITORY = 'dallay/cortex'
WORKFLOW = REPOSITORY + '/.github/workflows/ci.yml'
TARGET = 'aarch64-apple-darwin'


class ProvenanceError(ValueError):
    """Missing or unauthenticated build provenance."""


def verify(binary, record, bundle, commit, materials, toolchain):
    try:
        if not re.fullmatch(r'[0-9a-f]{40}', commit):
            raise ProvenanceError('source_commit')
        for path in (binary, record, bundle):
            if Path(path).is_symlink() or not Path(path).is_file():
                raise ProvenanceError('missing_or_symlink_subject')
        data = json.loads(Path(record).read_bytes())
        expected = {'schema_version': 1, 'repository': REPOSITORY,
                    'workflow': WORKFLOW, 'source_ref': 'refs/heads/main',
                    'source_commit': commit, 'target': TARGET,
                    'package': 'muninn-c1-exporter', 'features': ['cpu-f32'],
                    'toolchain': toolchain, 'materials': materials,
                    'binary_sha256': hashlib.sha256(Path(binary).read_bytes()).hexdigest()}
        if data != expected or not materials or not toolchain:
            raise ProvenanceError('build_record_mismatch')
        for subject in (binary, record):
            result = subprocess.run([
                'gh', 'attestation', 'verify', str(subject), '--bundle', str(bundle),
                '--repo', REPOSITORY, '--signer-repo', REPOSITORY,
                '--signer-workflow', WORKFLOW,
                '--cert-identity', 'https://github.com/' + WORKFLOW + '@refs/heads/main',
                '--source-ref', 'refs/heads/main',
                '--source-digest', commit, '--deny-self-hosted-runners'],
                stdin=subprocess.DEVNULL, stdout=subprocess.PIPE,
                stderr=subprocess.PIPE, timeout=120, check=False)
            if result.returncode != 0:
                raise ProvenanceError('attestation_verification_failed')
        return data
    except (OSError, ValueError, TypeError, subprocess.SubprocessError) as error:
        raise ProvenanceError('provenance_rejected: ' + str(error)) from error
