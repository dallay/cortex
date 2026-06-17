# Changelog

## [0.1.0](https://github.com/dallay/cortex/compare/v0.0.1...v0.1.0) (2026-06-17)


### Features

* add dual-layer request deduplication with provider token caching ([#123](https://github.com/dallay/cortex/issues/123)) ([768e2d8](https://github.com/dallay/cortex/commit/768e2d883b0902644ff4f6b256f778a7ea6ae6f3))
* add model alias domain model and SQLite repository (Part 1/2) ([#109](https://github.com/dallay/cortex/issues/109)) ([e2df057](https://github.com/dallay/cortex/commit/e2df057dd1c19f464bb12e333c3cf285b457a985))
* anthropic migration ([e6cd47a](https://github.com/dallay/cortex/commit/e6cd47a4c301aca9a1c1c069c636fc746140a5a2))
* **api-key:** add allowed models and providers fields with validation ([67bff7f](https://github.com/dallay/cortex/commit/67bff7fdc8ed4fb5fc2d39e9eeb6729f418c76f4))
* **api-key:** model/provider restrictions and validation (closes [#46](https://github.com/dallay/cortex/issues/46)) ([#93](https://github.com/dallay/cortex/issues/93)) ([4224e10](https://github.com/dallay/cortex/commit/4224e10bc0c5246b218f0cb0eadd337cf4657e3d))
* **api-keys:** add API key CRUD management ([#53](https://github.com/dallay/cortex/issues/53)) ([8069508](https://github.com/dallay/cortex/commit/806950834f417aaa23aef4305a40370b7aa3b5dd))
* **api-keys:** add provider validation and structured restriction errors ([b16a839](https://github.com/dallay/cortex/commit/b16a83972fa6caf0b0c5a879e76b339fe19f1c3b))
* **api-key:** typed scopes with canonical values and route enforcement ([#89](https://github.com/dallay/cortex/issues/89)) ([3042560](https://github.com/dallay/cortex/commit/304256065884e4a28c27d4e1b17582d4a10626d1))
* **auth:** complete security and authorization architecture ([#33](https://github.com/dallay/cortex/issues/33)) ([82e7352](https://github.com/dallay/cortex/commit/82e735231dc25accdb16fb205dad77468cbe6079))
* **auth:** first-run bootstrap system for Rook ([#55](https://github.com/dallay/cortex/issues/55)) ([c06d517](https://github.com/dallay/cortex/commit/c06d517105a307ece3626e2ceb60067b45452970))
* **bootstrap:** auto-generate in-memory setup token at startup ([10453a2](https://github.com/dallay/cortex/commit/10453a23e97eaf70d68a39a975f2e953529582de))
* **cache:** add dual-layer cache foundation and signature inspection (WU-1) ([#121](https://github.com/dallay/cortex/issues/121)) ([c05e5ee](https://github.com/dallay/cortex/commit/c05e5ee943ea013f57806104ada532b7e75b3c9b))
* **cache:** content-based cache keys with SHA-256 signatures (1/2) ([#106](https://github.com/dallay/cortex/issues/106)) ([5c25ed3](https://github.com/dallay/cortex/commit/5c25ed3032809176c1e3b71be8161c170f6b2ea4))
* **cache:** HTTP management API, config validation, and observability (2/2) ([#110](https://github.com/dallay/cortex/issues/110)) ([0ca01d2](https://github.com/dallay/cortex/commit/0ca01d2bfa2b3bf8f6c3d65f309637fe112377f4))
* **ci:** enhance CI scripts with fail-fast behavior and additional commands ([ff49b7a](https://github.com/dallay/cortex/commit/ff49b7ae69f6a94e15edcc2d72f8034fdf1b6c19))
* **dashboard:** add API key scopes, restrictions, and rotate UI ([fcc40a5](https://github.com/dallay/cortex/commit/fcc40a548cdd8db112afc7e1e3a5d2393522befc)), closes [#46](https://github.com/dallay/cortex/issues/46)
* **dashboard:** add auth route guards, login view, and session state ([#76](https://github.com/dallay/cortex/issues/76)) ([ac20a3a](https://github.com/dallay/cortex/commit/ac20a3a810645b512b13056ccaf24ee6efb4e6ae))
* **dashboard:** add Codecov bundle analysis plugin ([#129](https://github.com/dallay/cortex/issues/129)) ([48ca9df](https://github.com/dallay/cortex/commit/48ca9dff75a88fe239621bc8202199e2f46197bd))
* **dashboard:** add provider management UI with Ollama Cloud support ([#115](https://github.com/dallay/cortex/issues/115)) ([2327fd6](https://github.com/dallay/cortex/commit/2327fd60c9991435c62c45a334ba01a39ee00057))
* **dashboard:** branded provider icons and external title links ([#139](https://github.com/dallay/cortex/issues/139)) ([775439b](https://github.com/dallay/cortex/commit/775439b074bc07aabb18a1b8cf334975569aa795))
* **dashboard:** refactor providers UI to 3-screen catalog/details/connection hierarchy ([b1eb991](https://github.com/dallay/cortex/commit/b1eb9912a4cfd9df688becc5ce82be9b4d9b5d10))
* **dashboard:** scaffold dashboard with lazy-loaded navigation and API client ([fb6a8cc](https://github.com/dallay/cortex/commit/fb6a8ccf877e4d95160f7a3d72e9827ec2747a82))
* **db-migration:** SQLite migration system with Refinery ([#54](https://github.com/dallay/cortex/issues/54)) ([b97bd0f](https://github.com/dallay/cortex/commit/b97bd0fe9099bf6cadaca9a550f7f04cf7474ed3))
* **di:** remove TOML provider config, complete dynamic provider registry wiring ([062c88b](https://github.com/dallay/cortex/commit/062c88b094df7c8723bf6e92b4319a51c690fb49))
* Multi-step Fallback Chains (Combos) ([#104](https://github.com/dallay/cortex/issues/104)) ([409e1db](https://github.com/dallay/cortex/commit/409e1db1c0e06a9b4101cebbbd40a944be8c55e6))
* **providers-ollama:** 2-step health probe + hidden baseUrl for managed cloud ([f934acd](https://github.com/dallay/cortex/commit/f934acd3c4f853e723e412ec831d194ff8e7f81c))
* **providers:** add real quota dashboard ([a5d2f91](https://github.com/dallay/cortex/commit/a5d2f91f811d61b6389f14c8936cba37e2c3fd1d))
* **providers:** add real quota dashboard ([#155](https://github.com/dallay/cortex/issues/155)) ([d08c193](https://github.com/dallay/cortex/commit/d08c193740d1261dc82a50cf5bc2432b9df122fa))
* **providers:** add test-credentials endpoint to fix CONFLICT bug ([541f5ce](https://github.com/dallay/cortex/commit/541f5cef00d84486ffc7c66bdf95abcce73068f9))
* **providers:** add warning state to credential validation ([a411b50](https://github.com/dallay/cortex/commit/a411b50dbe7b96d7aae54888283595c2dd400667))
* **rate-limit:** Per-Client Rate Limiting with API Key Tiers, IP Limits, and Admin CRUD ([#95](https://github.com/dallay/cortex/issues/95)) ([7b3693e](https://github.com/dallay/cortex/commit/7b3693ecf8883a922d3b9ed5c8bf5039f3a921a5))
* **resilience:** expose circuit breaker state via HTTP endpoints ([#103](https://github.com/dallay/cortex/issues/103)) ([1cf1946](https://github.com/dallay/cortex/commit/1cf19466afa3b1cbaac4612f2912fc6b7a9c4f7d))
* **rook-core:** dynamic provider registry foundation (PR1) ([#26](https://github.com/dallay/cortex/issues/26)) ([a1b6f5c](https://github.com/dallay/cortex/commit/a1b6f5c9c7bc4038f84c644d5a2ff76982506ae2))
* **telemetry:** add API endpoints and configuration (2/2) ([#114](https://github.com/dallay/cortex/issues/114)) ([b5c903b](https://github.com/dallay/cortex/commit/b5c903b225eb668cc440d2371f819735ae03ab64))
* **telemetry:** add core request telemetry tracker foundation (1/2) ([#113](https://github.com/dallay/cortex/issues/113)) ([c8cad67](https://github.com/dallay/cortex/commit/c8cad67bfcb15c22eaf7ba679e3537ea73691a89))
* **translation:** add provider format translation layer ([#59](https://github.com/dallay/cortex/issues/59)) ([c3297f3](https://github.com/dallay/cortex/commit/c3297f3aa74ec331526560fb4f92ce7ec57b8d66))
* **translation:** FormatRegistry::register() for multi-format routing Phase 2 ([#68](https://github.com/dallay/cortex/issues/68)) ([b555f22](https://github.com/dallay/cortex/commit/b555f2296475ca1f18b39fe3b619cc0990d1c7e7)), closes [#63](https://github.com/dallay/cortex/issues/63)
* **translation:** preserve tool content across OpenAI/Anthropic adapters ([d0f7438](https://github.com/dallay/cortex/commit/d0f7438c7bdcc84a1e2d15b7c38ae4f5ebb9cf1b))
* **translation:** tool_use/tool_result message content Phase 2 ([#73](https://github.com/dallay/cortex/issues/73)) ([4cd52fe](https://github.com/dallay/cortex/commit/4cd52feca1264731c1e83ec61e5acd91aae3a6e6))
* update server port from 8080 to 3773 and adjust related configurations ([#147](https://github.com/dallay/cortex/issues/147)) ([eeb2798](https://github.com/dallay/cortex/commit/eeb27985bf290090351602c00d077a2208f56f8a))
* **usage:** token counts, cost estimation, usage history API, retention sweep ([#102](https://github.com/dallay/cortex/issues/102)) ([dcff073](https://github.com/dallay/cortex/commit/dcff073aaec4072d976e4983e534805f6da3efcb))
* **webkit csrf:** bundle csrf_token in POST /login and seed frontend cache ([cead68a](https://github.com/dallay/cortex/commit/cead68adae09f159054427c296cbffdc08dedda1))


### Bug Fixes

* bootstrap token, logout, endpoints duplication, and i18n keys ([#81](https://github.com/dallay/cortex/issues/81)) ([24fe36a](https://github.com/dallay/cortex/commit/24fe36ac5db6d7a314a28e111dffef7ad1cfc5f4))
* **build:** allow pre-built dashboard in release mode ([65412b9](https://github.com/dallay/cortex/commit/65412b9e4fb0fb338b152f1f0974627087a04c69))
* **build:** allow usage of pre-built dashboard artifacts in release mode ([fd54abe](https://github.com/dallay/cortex/commit/fd54abe2308a6ea8a37f73318d3187015a05aa66))
* **ci:** fix fmt and cross-compile jobs in GitHub Actions ([0bc4804](https://github.com/dallay/cortex/commit/0bc480406108174e8072181403d69e2b4f9ff73c))
* **ci:** resolve all code scanning security alerts ([#57](https://github.com/dallay/cortex/issues/57)) ([6cf446f](https://github.com/dallay/cortex/commit/6cf446f3a67870b2e96d26ededcce2c80360e6b4))
* **dashboard:** resolve SonarCloud quality gate failures ([cf950d1](https://github.com/dallay/cortex/commit/cf950d1c6c469c8eb9aa14af32be2ee2488d1bd5))
* **db:** ensure migrations run for in-memory databases and improve connection handling ([547f7cd](https://github.com/dallay/cortex/commit/547f7cd90f91b17284a3a2da69476bf5e2c6051a))
* **e2e:** use getByRole('checkbox') for reka-ui Checkbox + cookie scope fix ([#145](https://github.com/dallay/cortex/issues/145)) ([aa062b2](https://github.com/dallay/cortex/commit/aa062b2f2f638238cddf9fca33f8339514e727ab))
* populate provider supported_models from model catalog ([5a3ac0f](https://github.com/dallay/cortex/commit/5a3ac0f8321c8baf85d2dc537826f1bc81f36632))
* **providers:** link /providers/quota to tracking issue [#132](https://github.com/dallay/cortex/issues/132) ([54005ee](https://github.com/dallay/cortex/commit/54005ee06900ace1cdae71592cdcfe34b1902859))
* **providers:** restore es locale parity, tighten wire types, flatten component layout ([616d1d5](https://github.com/dallay/cortex/commit/616d1d5f685934480faf4742887d6773b8a51238))
* **quality:** resolve SonarQube code quality issues ([07792e3](https://github.com/dallay/cortex/commit/07792e3cd123eab9d0768517351a2529fbe94745))
* **quality:** resolve SonarQube code quality issues ([#117](https://github.com/dallay/cortex/issues/117)) ([9658a05](https://github.com/dallay/cortex/commit/9658a05c0db475c6ea24edba2a146de1a7535982))
* **quality:** resolve SonarQube issues across codebase ([#60](https://github.com/dallay/cortex/issues/60)) ([46d9e62](https://github.com/dallay/cortex/commit/46d9e62e8cf75071a4d0e184e226b0aa7fa83210))
* **quality:** resolve SonarQube Quality Gate failures ([#122](https://github.com/dallay/cortex/issues/122)) ([2932d8d](https://github.com/dallay/cortex/commit/2932d8d2d5d1f720537ca292b78ae1e7bbef181b))
* **rook:** resolve block_on deadlock, fix dashboard route, add Docker e2e infra ([#36](https://github.com/dallay/cortex/issues/36)) ([02dbafb](https://github.com/dallay/cortex/commit/02dbafb57084e20ee90b1473a5e9e746858501f1))
* **security-deep:** use aquasecurity/trivy-action instead of community install script ([#70](https://github.com/dallay/cortex/issues/70)) ([fa6466b](https://github.com/dallay/cortex/commit/fa6466b3c54a15c9df8f45588809af4411eeaf22))
* **security:** resolve open code scanning and Dependabot alerts ([#77](https://github.com/dallay/cortex/issues/77)) ([a4ad9d4](https://github.com/dallay/cortex/commit/a4ad9d4689e6dfc2c3852952644097c1388f6d37))
* **tests:** use std::env::temp_dir() for cross-platform temp DB path ([6147425](https://github.com/dallay/cortex/commit/6147425e55ca3e8e333437e82317df228d76a19b))


### Code Refactoring

* **dashboard:** move locale and theme selectors to header bar ([4b07441](https://github.com/dallay/cortex/commit/4b07441d695e8391c9f8bf26dfc52d2b9ec0bd0c))
* **dashboard:** remove unused imports and dead code from views ([#94](https://github.com/dallay/cortex/issues/94)) ([8a2b717](https://github.com/dallay/cortex/commit/8a2b7177968dbb51d5d17457061c4fdf28badc49))
* **providers:** use WireAuthType alias in AddProviderDialog helper ([01ffd1c](https://github.com/dallay/cortex/commit/01ffd1c06fb405a8352789b811e01a850e5ad5c8))
* reduce cognitive complexity in 8 functions to fix SonarCloud quality gate ([7a768b3](https://github.com/dallay/cortex/commit/7a768b3f41af5b280db69a33f6ae7fb48c660cdf))
* **specs:** rewrite provider-connections specs as technology-agnostic ([#2](https://github.com/dallay/cortex/issues/2)) ([7474a63](https://github.com/dallay/cortex/commit/7474a63cf137af55b98a2a00c1c7d18c761dc8d0))


### Tests

* add model passing tests to di_tests ([121327c](https://github.com/dallay/cortex/commit/121327cfedbc355a2b9e0a8020051736f6822a3c))
* **cache:** add e2e integration tests and archive read-cache change ([#112](https://github.com/dallay/cortex/issues/112)) ([6e6f831](https://github.com/dallay/cortex/commit/6e6f8316de188c7b9d2d591215c4fd947a75c32b))


### Chores

* **deps-rust:** bump dirs from 5.0.1 to 6.0.0 ([#14](https://github.com/dallay/cortex/issues/14)) ([36d59ac](https://github.com/dallay/cortex/commit/36d59ac80724e7ed0f9ee69af02a09eb98023191))
* **deps-rust:** bump toml from 0.8.23 to 1.1.2+spec-1.1.0 ([#7](https://github.com/dallay/cortex/issues/7)) ([b52a6d5](https://github.com/dallay/cortex/commit/b52a6d553a41ef256c5f1f9f91bfb686cd058408))
* **deps:** update pnpm, markdownlint-cli2, vue-router, and devDependencies to latest versions ([bf3d969](https://github.com/dallay/cortex/commit/bf3d969b5418ab0dd329494a2a857b818da00590))
* initial scaffold ([7da1a73](https://github.com/dallay/cortex/commit/7da1a7359d873c0b5d979e82b1f266bf453674d1))
* **tests:** reformat assertions and function calls for improved readability ([d4e33ad](https://github.com/dallay/cortex/commit/d4e33ad929d1815e2489038f9d76d7cd541aa40d))
