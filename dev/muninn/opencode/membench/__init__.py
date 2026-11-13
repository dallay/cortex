"""Muninn memory benchmark harness (OpenCode in Docker).

The harness orchestrates a disposable experiment: a seeded Muninn instance, an
OpenCode agent with least-privilege permissions, and a fixed set of evaluation
modes and memory conditions. It never asserts global memory parity; it reports
observable per-run evidence and a diagnosis of where a failure happened.
"""

__all__ = ["__version__"]

__version__ = "0.1.0"
