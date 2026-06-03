# Changelog

## [0.1.0](https://github.com/dallay/cortex/compare/v0.0.1...v0.1.0) (2026-06-03)


### Features

* **api-key:** add allowed models and providers fields with validation ([67bff7f](https://github.com/dallay/cortex/commit/67bff7fdc8ed4fb5fc2d39e9eeb6729f418c76f4))
* **api-key:** model/provider restrictions and validation (closes [#46](https://github.com/dallay/cortex/issues/46)) ([#93](https://github.com/dallay/cortex/issues/93)) ([4224e10](https://github.com/dallay/cortex/commit/4224e10bc0c5246b218f0cb0eadd337cf4657e3d))
* **api-keys:** add API key CRUD management ([#53](https://github.com/dallay/cortex/issues/53)) ([8069508](https://github.com/dallay/cortex/commit/806950834f417aaa23aef4305a40370b7aa3b5dd))
* **api-keys:** add provider validation and structured restriction errors ([b16a839](https://github.com/dallay/cortex/commit/b16a83972fa6caf0b0c5a879e76b339fe19f1c3b))
* **api-key:** typed scopes with canonical values and route enforcement ([#89](https://github.com/dallay/cortex/issues/89)) ([3042560](https://github.com/dallay/cortex/commit/304256065884e4a28c27d4e1b17582d4a10626d1))
* **auth:** complete security and authorization architecture ([#33](https://github.com/dallay/cortex/issues/33)) ([82e7352](https://github.com/dallay/cortex/commit/82e735231dc25accdb16fb205dad77468cbe6079))
* **auth:** first-run bootstrap system for Rook ([#55](https://github.com/dallay/cortex/issues/55)) ([c06d517](https://github.com/dallay/cortex/commit/c06d517105a307ece3626e2ceb60067b45452970))
* **bootstrap:** auto-generate in-memory setup token at startup ([10453a2](https://github.com/dallay/cortex/commit/10453a23e97eaf70d68a39a975f2e953529582de))
* **dashboard:** add API key scopes, restrictions, and rotate UI ([fcc40a5](https://github.com/dallay/cortex/commit/fcc40a548cdd8db112afc7e1e3a5d2393522befc)), closes [#46](https://github.com/dallay/cortex/issues/46)
* **dashboard:** add auth route guards, login view, and session state ([#76](https://github.com/dallay/cortex/issues/76)) ([ac20a3a](https://github.com/dallay/cortex/commit/ac20a3a810645b512b13056ccaf24ee6efb4e6ae))
* **dashboard:** scaffold dashboard with lazy-loaded navigation and API client ([fb6a8cc](https://github.com/dallay/cortex/commit/fb6a8ccf877e4d95160f7a3d72e9827ec2747a82))
* **db-migration:** SQLite migration system with Refinery ([#54](https://github.com/dallay/cortex/issues/54)) ([b97bd0f](https://github.com/dallay/cortex/commit/b97bd0fe9099bf6cadaca9a550f7f04cf7474ed3))
* **di:** remove TOML provider config, complete dynamic provider registry wiring ([062c88b](https://github.com/dallay/cortex/commit/062c88b094df7c8723bf6e92b4319a51c690fb49))
* **rate-limit:** Per-Client Rate Limiting with API Key Tiers, IP Limits, and Admin CRUD ([#95](https://github.com/dallay/cortex/issues/95)) ([7b3693e](https://github.com/dallay/cortex/commit/7b3693ecf8883a922d3b9ed5c8bf5039f3a921a5))
* **rook-core:** dynamic provider registry foundation (PR1) ([#26](https://github.com/dallay/cortex/issues/26)) ([a1b6f5c](https://github.com/dallay/cortex/commit/a1b6f5c9c7bc4038f84c644d5a2ff76982506ae2))
* **translation:** add provider format translation layer ([#59](https://github.com/dallay/cortex/issues/59)) ([c3297f3](https://github.com/dallay/cortex/commit/c3297f3aa74ec331526560fb4f92ce7ec57b8d66))
* **translation:** FormatRegistry::register() for multi-format routing Phase 2 ([#68](https://github.com/dallay/cortex/issues/68)) ([b555f22](https://github.com/dallay/cortex/commit/b555f2296475ca1f18b39fe3b619cc0990d1c7e7)), closes [#63](https://github.com/dallay/cortex/issues/63)
* **translation:** preserve tool content across OpenAI/Anthropic adapters ([d0f7438](https://github.com/dallay/cortex/commit/d0f7438c7bdcc84a1e2d15b7c38ae4f5ebb9cf1b))
* **translation:** tool_use/tool_result message content Phase 2 ([#73](https://github.com/dallay/cortex/issues/73)) ([4cd52fe](https://github.com/dallay/cortex/commit/4cd52feca1264731c1e83ec61e5acd91aae3a6e6))
* **webkit csrf:** bundle csrf_token in POST /login and seed frontend cache ([cead68a](https://github.com/dallay/cortex/commit/cead68adae09f159054427c296cbffdc08dedda1))


### Bug Fixes

* bootstrap token, logout, endpoints duplication, and i18n keys ([#81](https://github.com/dallay/cortex/issues/81)) ([24fe36a](https://github.com/dallay/cortex/commit/24fe36ac5db6d7a314a28e111dffef7ad1cfc5f4))
* **ci:** fix fmt and cross-compile jobs in GitHub Actions ([0bc4804](https://github.com/dallay/cortex/commit/0bc480406108174e8072181403d69e2b4f9ff73c))
* **ci:** resolve all code scanning security alerts ([#57](https://github.com/dallay/cortex/issues/57)) ([6cf446f](https://github.com/dallay/cortex/commit/6cf446f3a67870b2e96d26ededcce2c80360e6b4))
* **quality:** resolve SonarQube issues across codebase ([#60](https://github.com/dallay/cortex/issues/60)) ([46d9e62](https://github.com/dallay/cortex/commit/46d9e62e8cf75071a4d0e184e226b0aa7fa83210))
* **rook:** resolve block_on deadlock, fix dashboard route, add Docker e2e infra ([#36](https://github.com/dallay/cortex/issues/36)) ([02dbafb](https://github.com/dallay/cortex/commit/02dbafb57084e20ee90b1473a5e9e746858501f1))
* **security-deep:** use aquasecurity/trivy-action instead of community install script ([#70](https://github.com/dallay/cortex/issues/70)) ([fa6466b](https://github.com/dallay/cortex/commit/fa6466b3c54a15c9df8f45588809af4411eeaf22))
* **security:** resolve open code scanning and Dependabot alerts ([#77](https://github.com/dallay/cortex/issues/77)) ([a4ad9d4](https://github.com/dallay/cortex/commit/a4ad9d4689e6dfc2c3852952644097c1388f6d37))


### Code Refactoring

* **dashboard:** move locale and theme selectors to header bar ([4b07441](https://github.com/dallay/cortex/commit/4b07441d695e8391c9f8bf26dfc52d2b9ec0bd0c))
* **dashboard:** remove unused imports and dead code from views ([#94](https://github.com/dallay/cortex/issues/94)) ([8a2b717](https://github.com/dallay/cortex/commit/8a2b7177968dbb51d5d17457061c4fdf28badc49))
* **specs:** rewrite provider-connections specs as technology-agnostic ([#2](https://github.com/dallay/cortex/issues/2)) ([7474a63](https://github.com/dallay/cortex/commit/7474a63cf137af55b98a2a00c1c7d18c761dc8d0))


### Chores

* **deps-rust:** bump dirs from 5.0.1 to 6.0.0 ([#14](https://github.com/dallay/cortex/issues/14)) ([36d59ac](https://github.com/dallay/cortex/commit/36d59ac80724e7ed0f9ee69af02a09eb98023191))
* **deps-rust:** bump toml from 0.8.23 to 1.1.2+spec-1.1.0 ([#7](https://github.com/dallay/cortex/issues/7)) ([b52a6d5](https://github.com/dallay/cortex/commit/b52a6d553a41ef256c5f1f9f91bfb686cd058408))
* **deps:** update pnpm, markdownlint-cli2, vue-router, and devDependencies to latest versions ([bf3d969](https://github.com/dallay/cortex/commit/bf3d969b5418ab0dd329494a2a857b818da00590))
* initial scaffold ([7da1a73](https://github.com/dallay/cortex/commit/7da1a7359d873c0b5d979e82b1f266bf453674d1))
* **tests:** reformat assertions and function calls for improved readability ([d4e33ad](https://github.com/dallay/cortex/commit/d4e33ad929d1815e2489038f9d76d7cd541aa40d))
