# Changelog

## [0.1.0](https://github.com/dallay/cortex/compare/huginn@v0.0.1...huginn@v0.1.0) (2026-10-30)


### Features

* add standalone Huginn provider login ([#417](https://github.com/dallay/cortex/issues/417)) ([df670e7](https://github.com/dallay/cortex/commit/df670e728a6b274c178744025ca2a13f8263fb50))
* **agent:** establish authorable plugin contract and UI contribution lifecycle ([#329](https://github.com/dallay/cortex/issues/329)) ([a80858e](https://github.com/dallay/cortex/commit/a80858ec13654c93f9cda105cb3469da5010ba32))
* **dashboard:** render application name and version dynamically ([#339](https://github.com/dallay/cortex/issues/339)) ([205053e](https://github.com/dallay/cortex/commit/205053e98c73ebb6dd8623ad989689371ba4adb4))
* harden Huginn boundaries and add Makefile workflow ([#338](https://github.com/dallay/cortex/issues/338)) ([80e352f](https://github.com/dallay/cortex/commit/80e352fd67459fa4e8a314852a3ef76ba3612bb6))
* **huginn:** add OpenAI-compatible adapters and verified local Ollama ([#358](https://github.com/dallay/cortex/issues/358)) ([916e36d](https://github.com/dallay/cortex/commit/916e36dc11e1bea40939cf94f86000540b4387ac))
* **huginn:** add optional Providers UI ([#361](https://github.com/dallay/cortex/issues/361)) ([050bf75](https://github.com/dallay/cortex/commit/050bf75124cd0d56c04dcc56b958b981af6d61db))
* **huginn:** add overlay system with persistent status bar ([#383](https://github.com/dallay/cortex/issues/383)) ([e80c6fd](https://github.com/dallay/cortex/commit/e80c6fda1374c9fb29069420acecdb40b3a3e169))
* **huginn:** add scoped prompt templates ([#439](https://github.com/dallay/cortex/issues/439)) ([4e7d283](https://github.com/dallay/cortex/commit/4e7d283e33676b2a352f63e67fe86b868ae81701))
* **huginn:** add transactional provider substitution ([#516](https://github.com/dallay/cortex/issues/516)) ([d24555d](https://github.com/dallay/cortex/commit/d24555d15114faf94adf93e8215fd8a6c40dd4bc))
* **huginn:** discover and inspect scoped agent skills ([#433](https://github.com/dallay/cortex/issues/433)) ([52fb617](https://github.com/dallay/cortex/commit/52fb6171ba29221596e91ca6fc808e20f8396776))
* **huginn:** extract application layer and enforce plugin boundaries ([#323](https://github.com/dallay/cortex/issues/323)) ([4944e09](https://github.com/dallay/cortex/commit/4944e092507bc94c063bc28d2b8236db47a189b0))
* **huginn:** introduce Ratatui-first TUI as kernel-managed presentation plugin ([#320](https://github.com/dallay/cortex/issues/320)) ([4ff976d](https://github.com/dallay/cortex/commit/4ff976d8f56be16ee16ab4315d97a03228da79c6))
* **huginn:** invoke skills explicitly with /skill:name ([#458](https://github.com/dallay/cortex/issues/458)) ([98a664d](https://github.com/dallay/cortex/commit/98a664de53bb60de89c4dad7c8f23484b1934703))
* **huginn:** load an external WASM Rust outline component ([#508](https://github.com/dallay/cortex/issues/508)) ([10b2a4e](https://github.com/dallay/cortex/commit/10b2a4e2efb866f22072ecd5426ff2fc30403074))
* **huginn:** load skill instructions on demand ([#438](https://github.com/dallay/cortex/issues/438)) ([a7c75e5](https://github.com/dallay/cortex/commit/a7c75e5d96a6d55f72373f3f15fe39ef64961089))
* **huginn:** P0.5 Providers integration tests and safe-recovery ([#366](https://github.com/dallay/cortex/issues/366)) ([13f0d0c](https://github.com/dallay/cortex/commit/13f0d0c25c2542c0c2c1ed3a1ef0177ef422dfef))
* **huginn:** streamline searchable transient provider onboarding ([#401](https://github.com/dallay/cortex/issues/401)) ([5877b5f](https://github.com/dallay/cortex/commit/5877b5f09187568e7d347a448b17ce11e470b8ec))
* unify Rook and Huginn home directories ([#492](https://github.com/dallay/cortex/issues/492)) ([dd5bbef](https://github.com/dallay/cortex/commit/dd5bbef7ea5cd2024b805bdeeb89332f3e638e14))


### Bug Fixes

* address SonarCloud findings ([#398](https://github.com/dallay/cortex/issues/398)) ([77bd01c](https://github.com/dallay/cortex/commit/77bd01c0939870307193985601910901ca43cf6f))
* **deps-rust:** update rust dependencies ([#378](https://github.com/dallay/cortex/issues/378)) ([1843cdf](https://github.com/dallay/cortex/commit/1843cdf55cebe24997709e330d94d415e6b1d738))
* **huginn:** allow first-run without configured model ([#340](https://github.com/dallay/cortex/issues/340)) ([3be1190](https://github.com/dallay/cortex/commit/3be1190b31e1c52bd5b9ceea4ca096cb1c36918a))
* **huginn:** harden substitution lifecycle after PR [#516](https://github.com/dallay/cortex/issues/516) ([#518](https://github.com/dallay/cortex/issues/518)) ([4604d98](https://github.com/dallay/cortex/commit/4604d9879b0cc5a62c0742e47162dd5fedb57ad0))
* **security:** auditar excepciones puntuales de rutas ([#478](https://github.com/dallay/cortex/issues/478)) ([565e633](https://github.com/dallay/cortex/commit/565e633706ba778179ed32e8de4511ea0be546cf))
* **security:** remediate code scanning findings ([#427](https://github.com/dallay/cortex/issues/427)) ([ec991fc](https://github.com/dallay/cortex/commit/ec991fc01cd02ce5616f0bb1cf0c85653dd42348))


### Code Refactoring

* **huginn:** rename personal coding agent from agent to huginn ([#315](https://github.com/dallay/cortex/issues/315)) ([646d0d5](https://github.com/dallay/cortex/commit/646d0d5249ad88bf02dac97111d95cb97866940b))


### Tests

* **huginn:** S1 skills and template acceptance coverage ([#462](https://github.com/dallay/cortex/issues/462)) ([f40a3ea](https://github.com/dallay/cortex/commit/f40a3eacf6cc8a583222a5e2daeb6e040b3e55dc))
* **huginn:** stabilize multiline TUI integration test ([#515](https://github.com/dallay/cortex/issues/515)) ([f5dbe60](https://github.com/dallay/cortex/commit/f5dbe6089de2c1106bcb03b32341b85550ec60ea))


### Chores

* code-quality hardening (dead code, lock poisoning, Anthropic stream fix, dashboard typecheck) ([#425](https://github.com/dallay/cortex/issues/425)) ([2178a45](https://github.com/dallay/cortex/commit/2178a4580b379aa7a8c5db76c786b4b4ef30795a))
