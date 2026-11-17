# Muninn — Attribution and References

Muninn is an independent Rust implementation intended to reproduce the
observable behavior of **Neo4j Agent Memory**.

## References

- `neo4j-labs/agent-memory`, revision
  `9d45af5529ecb5f2fe352013468a11087f3163d4`, is licensed under the MIT License.
  Copyright © 2026 Neo4j, Inc.
- `neo4j-labs/agent-memory-tck`, revision
  `e60b410f1046dc21db50f9aba48e171dc8a9b5a1`, is licensed under the MIT License.
- Copies of both upstream license texts are retained in
  [`licenses/agent-memory-LICENSE.txt`](licenses/agent-memory-LICENSE.txt) and
  [`licenses/agent-memory-tck-LICENSE.txt`](licenses/agent-memory-tck-LICENSE.txt).
- `candle-core`, version 0.11.0 (registry SHA-256
  `227eb9b2469de76de99d48f649903f54744a0190f6fb9f903a72c524f9f458312`),
  uses the MIT license option. The CPU/F32 patch in `third_party/candle-core`
  retains its license and has [a separate notice](licenses/CANDLE-NOTICE.md).
  The [license copy](licenses/candle-0.11.0-MIT.txt) accompanies the spike.

The Rust implementation does not include upstream Python code or upstream
comments. Python files under `dev/muninn/` are local adapters and oracle
capture/verification tools, not a copy of the upstream package.
