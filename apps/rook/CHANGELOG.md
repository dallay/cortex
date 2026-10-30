# Changelog

All notable Rook releases are documented in this file.

The file is maintained by Release Please. Rook releases use the independent
`rook@vX.Y.Z` tag and never imply a repository-wide Cortex version.

## [0.1.0](https://github.com/dallay/cortex/compare/rook@v0.0.1...rook@v0.1.0) (2026-10-30)


### Features

* add dual-layer request deduplication with provider token caching ([#123](https://github.com/dallay/cortex/issues/123)) ([768e2d8](https://github.com/dallay/cortex/commit/768e2d883b0902644ff4f6b256f778a7ea6ae6f3))
* add model alias domain model and SQLite repository (Part 1/2) ([#109](https://github.com/dallay/cortex/issues/109)) ([e2df057](https://github.com/dallay/cortex/commit/e2df057dd1c19f464bb12e333c3cf285b457a985))
* add secure OpenAI-compatible providers ([#421](https://github.com/dallay/cortex/issues/421)) ([16bdad9](https://github.com/dallay/cortex/commit/16bdad94832e67a4e68e37ec0baac1b58c13a595))
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
* **dashboard:** complete admin API parity ([#437](https://github.com/dallay/cortex/issues/437)) ([2980e12](https://github.com/dallay/cortex/commit/2980e12ee828e6df9594c1dac899ea0abb39d978))
* **dashboard:** refactor providers UI to 3-screen catalog/details/connection hierarchy ([b1eb991](https://github.com/dallay/cortex/commit/b1eb9912a4cfd9df688becc5ce82be9b4d9b5d10))
* **dashboard:** render application name and version dynamically ([#339](https://github.com/dallay/cortex/issues/339)) ([205053e](https://github.com/dallay/cortex/commit/205053e98c73ebb6dd8623ad989689371ba4adb4))
* **dashboard:** scaffold dashboard with lazy-loaded navigation and API client ([fb6a8cc](https://github.com/dallay/cortex/commit/fb6a8ccf877e4d95160f7a3d72e9827ec2747a82))
* **db-migration:** SQLite migration system with Refinery ([#54](https://github.com/dallay/cortex/issues/54)) ([b97bd0f](https://github.com/dallay/cortex/commit/b97bd0fe9099bf6cadaca9a550f7f04cf7474ed3))
* **di:** remove TOML provider config, complete dynamic provider registry wiring ([062c88b](https://github.com/dallay/cortex/commit/062c88b094df7c8723bf6e92b4319a51c690fb49))
* enable independent Rook and Huginn releases with Release Please ([#333](https://github.com/dallay/cortex/issues/333)) ([48e3536](https://github.com/dallay/cortex/commit/48e3536cf95f9c2ab765800c2c4fe2fe5ebb427e))
* harden Huginn boundaries and add Makefile workflow ([#338](https://github.com/dallay/cortex/issues/338)) ([80e352f](https://github.com/dallay/cortex/commit/80e352fd67459fa4e8a314852a3ef76ba3612bb6))
* **huginn:** add workspace file index ([#344](https://github.com/dallay/cortex/issues/344)) ([59d30dd](https://github.com/dallay/cortex/commit/59d30dd93cb00608c86b9243a29ebeddf6163a0e))
* Multi-step Fallback Chains (Combos) ([#104](https://github.com/dallay/cortex/issues/104)) ([409e1db](https://github.com/dallay/cortex/commit/409e1db1c0e06a9b4101cebbbd40a944be8c55e6))
* **oauth:** add provider token lifecycle ([#341](https://github.com/dallay/cortex/issues/341)) ([f596566](https://github.com/dallay/cortex/commit/f5965667d5158b0bdfb2b2e422e055cda3b5f10d))
* **ollama:** preservar metadatos de respuesta y llamadas nativas ([#319](https://github.com/dallay/cortex/issues/319)) ([018b464](https://github.com/dallay/cortex/commit/018b464debdddd6b9ddf7b6bb3364448d156c5e1))
* **providers-ollama:** 2-step health probe + hidden baseUrl for managed cloud ([f934acd](https://github.com/dallay/cortex/commit/f934acd3c4f853e723e412ec831d194ff8e7f81c))
* **providers:** add real quota dashboard ([a5d2f91](https://github.com/dallay/cortex/commit/a5d2f91f811d61b6389f14c8936cba37e2c3fd1d))
* **providers:** add real quota dashboard ([#155](https://github.com/dallay/cortex/issues/155)) ([d08c193](https://github.com/dallay/cortex/commit/d08c193740d1261dc82a50cf5bc2432b9df122fa))
* **providers:** add test-credentials endpoint to fix CONFLICT bug ([541f5ce](https://github.com/dallay/cortex/commit/541f5cef00d84486ffc7c66bdf95abcce73068f9))
* **providers:** add warning state to credential validation ([a411b50](https://github.com/dallay/cortex/commit/a411b50dbe7b96d7aae54888283595c2dd400667))
* **quota:** capture provider quota headers with persistence and HTTP wiring ([#330](https://github.com/dallay/cortex/issues/330)) ([0d2cc04](https://github.com/dallay/cortex/commit/0d2cc04b1914ce7a264f190ce2c6fabc2284b3a7))
* **rate-limit:** Per-Client Rate Limiting with API Key Tiers, IP Limits, and Admin CRUD ([#95](https://github.com/dallay/cortex/issues/95)) ([7b3693e](https://github.com/dallay/cortex/commit/7b3693ecf8883a922d3b9ed5c8bf5039f3a921a5))
* **resilience:** expose circuit breaker state via HTTP endpoints ([#103](https://github.com/dallay/cortex/issues/103)) ([1cf1946](https://github.com/dallay/cortex/commit/1cf19466afa3b1cbaac4612f2912fc6b7a9c4f7d))
* **rook-core:** dynamic provider registry foundation (PR1) ([#26](https://github.com/dallay/cortex/issues/26)) ([a1b6f5c](https://github.com/dallay/cortex/commit/a1b6f5c9c7bc4038f84c644d5a2ff76982506ae2))
* **rook:** add capability-aware model catalog ([#466](https://github.com/dallay/cortex/issues/466)) ([271bd42](https://github.com/dallay/cortex/commit/271bd4242f4a48523bf972574d6c13ef86ff16fb))
* **rook:** add truthful provider usage dashboard ([#371](https://github.com/dallay/cortex/issues/371)) ([26e719a](https://github.com/dallay/cortex/commit/26e719a91fcf3019e1f242ec96828da4a02e7af8))
* **rook:** enforce configurable per-API-key spending budgets (DALLAY-686) ([#517](https://github.com/dallay/cortex/issues/517)) ([e6e5397](https://github.com/dallay/cortex/commit/e6e5397c75b656652ea35459004464e06cb0cb79))
* **rook:** normalize model aliases before combo routing ([#324](https://github.com/dallay/cortex/issues/324)) ([a17a6e7](https://github.com/dallay/cortex/commit/a17a6e70639f401df85cf224a7b086aa77a290b9))
* **rook:** virtual combo models, Anthropic tool-calling and gateway e2e ([#362](https://github.com/dallay/cortex/issues/362)) ([bc9b105](https://github.com/dallay/cortex/commit/bc9b105244fb01eead584cee8e77bc59fe9b3e33))
* **telemetry:** add API endpoints and configuration (2/2) ([#114](https://github.com/dallay/cortex/issues/114)) ([b5c903b](https://github.com/dallay/cortex/commit/b5c903b225eb668cc440d2371f819735ae03ab64))
* **telemetry:** add core request telemetry tracker foundation (1/2) ([#113](https://github.com/dallay/cortex/issues/113)) ([c8cad67](https://github.com/dallay/cortex/commit/c8cad67bfcb15c22eaf7ba679e3537ea73691a89))
* **translation:** add provider format translation layer ([#59](https://github.com/dallay/cortex/issues/59)) ([c3297f3](https://github.com/dallay/cortex/commit/c3297f3aa74ec331526560fb4f92ce7ec57b8d66))
* **translation:** FormatRegistry::register() for multi-format routing Phase 2 ([#68](https://github.com/dallay/cortex/issues/68)) ([b555f22](https://github.com/dallay/cortex/commit/b555f2296475ca1f18b39fe3b619cc0990d1c7e7)), closes [#63](https://github.com/dallay/cortex/issues/63)
* **translation:** preserve tool content across OpenAI/Anthropic adapters ([d0f7438](https://github.com/dallay/cortex/commit/d0f7438c7bdcc84a1e2d15b7c38ae4f5ebb9cf1b))
* **translation:** tool_use/tool_result message content Phase 2 ([#73](https://github.com/dallay/cortex/issues/73)) ([4cd52fe](https://github.com/dallay/cortex/commit/4cd52feca1264731c1e83ec61e5acd91aae3a6e6))
* unify Rook and Huginn home directories ([#492](https://github.com/dallay/cortex/issues/492)) ([dd5bbef](https://github.com/dallay/cortex/commit/dd5bbef7ea5cd2024b805bdeeb89332f3e638e14))
* update server port from 8080 to 3773 and adjust related configurations ([#147](https://github.com/dallay/cortex/issues/147)) ([eeb2798](https://github.com/dallay/cortex/commit/eeb27985bf290090351602c00d077a2208f56f8a))
* **usage:** token counts, cost estimation, usage history API, retention sweep ([#102](https://github.com/dallay/cortex/issues/102)) ([dcff073](https://github.com/dallay/cortex/commit/dcff073aaec4072d976e4983e534805f6da3efcb))
* **webkit csrf:** bundle csrf_token in POST /login and seed frontend cache ([cead68a](https://github.com/dallay/cortex/commit/cead68adae09f159054427c296cbffdc08dedda1))


### Bug Fixes

* address SonarCloud findings ([#398](https://github.com/dallay/cortex/issues/398)) ([77bd01c](https://github.com/dallay/cortex/commit/77bd01c0939870307193985601910901ca43cf6f))
* bootstrap token, logout, endpoints duplication, and i18n keys ([#81](https://github.com/dallay/cortex/issues/81)) ([24fe36a](https://github.com/dallay/cortex/commit/24fe36ac5db6d7a314a28e111dffef7ad1cfc5f4))
* **build:** allow pre-built dashboard in release mode ([65412b9](https://github.com/dallay/cortex/commit/65412b9e4fb0fb338b152f1f0974627087a04c69))
* **build:** allow usage of pre-built dashboard artifacts in release mode ([fd54abe](https://github.com/dallay/cortex/commit/fd54abe2308a6ea8a37f73318d3187015a05aa66))
* **ci:** fix fmt and cross-compile jobs in GitHub Actions ([0bc4804](https://github.com/dallay/cortex/commit/0bc480406108174e8072181403d69e2b4f9ff73c))
* **ci:** resolve all code scanning security alerts ([#57](https://github.com/dallay/cortex/issues/57)) ([6cf446f](https://github.com/dallay/cortex/commit/6cf446f3a67870b2e96d26ededcce2c80360e6b4))
* **dashboard:** associate catalog search input with label ([#282](https://github.com/dallay/cortex/issues/282)) ([8c1dfa3](https://github.com/dallay/cortex/commit/8c1dfa35211d2aa1abf0b705b501c6ba8ed6e0a1))
* **dashboard:** associate labels with form controls ([#471](https://github.com/dallay/cortex/issues/471)) ([ce0d09e](https://github.com/dallay/cortex/commit/ce0d09ed3f33b6abe095d1b2e3a7c4df255acb43))
* **dashboard:** remove duplicate id on Select component ([#348](https://github.com/dallay/cortex/issues/348)) ([2996d27](https://github.com/dallay/cortex/commit/2996d274f8caa5c8e639b98929ef6512d9492f36))
* **dashboard:** resolve SonarCloud quality gate failures ([cf950d1](https://github.com/dallay/cortex/commit/cf950d1c6c469c8eb9aa14af32be2ee2488d1bd5))
* **db:** ensure migrations run for in-memory databases and improve connection handling ([547f7cd](https://github.com/dallay/cortex/commit/547f7cd90f91b17284a3a2da69476bf5e2c6051a))
* **deps-js:** pin dependencies ([#265](https://github.com/dallay/cortex/issues/265)) ([7547d33](https://github.com/dallay/cortex/commit/7547d33ebb93658a327336b79e295778449d749c))
* **deps-js:** update javascript major ([#380](https://github.com/dallay/cortex/issues/380)) ([b2f1cbb](https://github.com/dallay/cortex/commit/b2f1cbb15bff5bd3fefd633cbfdceeeefabec33a))
* **deps-rust:** update rust dependencies ([#378](https://github.com/dallay/cortex/issues/378)) ([1843cdf](https://github.com/dallay/cortex/commit/1843cdf55cebe24997709e330d94d415e6b1d738))
* **deps-rust:** update rust major ([#291](https://github.com/dallay/cortex/issues/291)) ([bb8cf2d](https://github.com/dallay/cortex/commit/bb8cf2dfccc657a7a5cca0af7dcaffb55d45dbcc))
* **deps:** restore trusted lefthook release ([#499](https://github.com/dallay/cortex/issues/499)) ([c2e3d1c](https://github.com/dallay/cortex/commit/c2e3d1c54e5d097186971aa51b0da0b4533e8f86))
* **e2e:** use getByRole('checkbox') for reka-ui Checkbox + cookie scope fix ([#145](https://github.com/dallay/cortex/issues/145)) ([aa062b2](https://github.com/dallay/cortex/commit/aa062b2f2f638238cddf9fca33f8339514e727ab))
* **huginn:** allow first-run without configured model ([#340](https://github.com/dallay/cortex/issues/340)) ([3be1190](https://github.com/dallay/cortex/commit/3be1190b31e1c52bd5b9ceea4ca096cb1c36918a))
* populate provider supported_models from model catalog ([5a3ac0f](https://github.com/dallay/cortex/commit/5a3ac0f8321c8baf85d2dc537826f1bc81f36632))
* **providers:** link /providers/quota to tracking issue [#132](https://github.com/dallay/cortex/issues/132) ([54005ee](https://github.com/dallay/cortex/commit/54005ee06900ace1cdae71592cdcfe34b1902859))
* **providers:** restore es locale parity, tighten wire types, flatten component layout ([616d1d5](https://github.com/dallay/cortex/commit/616d1d5f685934480faf4742887d6773b8a51238))
* **quality:** resolve SonarQube code quality issues ([07792e3](https://github.com/dallay/cortex/commit/07792e3cd123eab9d0768517351a2529fbe94745))
* **quality:** resolve SonarQube code quality issues ([#117](https://github.com/dallay/cortex/issues/117)) ([9658a05](https://github.com/dallay/cortex/commit/9658a05c0db475c6ea24edba2a146de1a7535982))
* **quality:** resolve SonarQube issues across codebase ([#60](https://github.com/dallay/cortex/issues/60)) ([46d9e62](https://github.com/dallay/cortex/commit/46d9e62e8cf75071a4d0e184e226b0aa7fa83210))
* **quality:** resolve SonarQube Quality Gate failures ([#122](https://github.com/dallay/cortex/issues/122)) ([2932d8d](https://github.com/dallay/cortex/commit/2932d8d2d5d1f720537ca292b78ae1e7bbef181b))
* resolve Sonar gate and Windows clippy ([#477](https://github.com/dallay/cortex/issues/477)) ([c1f28f1](https://github.com/dallay/cortex/commit/c1f28f15dd697eec4e4f709a34838dce205f2a86))
* resolve SonarCloud quality gate issues ([#304](https://github.com/dallay/cortex/issues/304)) ([e0dacca](https://github.com/dallay/cortex/commit/e0daccaefe21ca5a6c781c9dfc513a34a8430f91))
* **rook:** aplicar default_combo y advertir token_cache no soportado ([#465](https://github.com/dallay/cortex/issues/465)) ([3c611ba](https://github.com/dallay/cortex/commit/3c611bada8f80888358bd24c5704649fc8623fe6))
* **rook:** resolve block_on deadlock, fix dashboard route, add Docker e2e infra ([#36](https://github.com/dallay/cortex/issues/36)) ([02dbafb](https://github.com/dallay/cortex/commit/02dbafb57084e20ee90b1473a5e9e746858501f1))
* **rook:** resolve secret path aliases ([#468](https://github.com/dallay/cortex/issues/468)) ([548fec0](https://github.com/dallay/cortex/commit/548fec08b11ab624ec7887c432d40e0f09046184))
* **security-deep:** use aquasecurity/trivy-action instead of community install script ([#70](https://github.com/dallay/cortex/issues/70)) ([fa6466b](https://github.com/dallay/cortex/commit/fa6466b3c54a15c9df8f45588809af4411eeaf22))
* **security:** auditar excepciones puntuales de rutas ([#478](https://github.com/dallay/cortex/issues/478)) ([565e633](https://github.com/dallay/cortex/commit/565e633706ba778179ed32e8de4511ea0be546cf))
* **security:** harden Rook rate limits and provider endpoints ([#349](https://github.com/dallay/cortex/issues/349)) ([fb4ebd6](https://github.com/dallay/cortex/commit/fb4ebd66797087331ded8729d1581e43bfe2189e))
* **security:** remediate code scanning findings ([#427](https://github.com/dallay/cortex/issues/427)) ([ec991fc](https://github.com/dallay/cortex/commit/ec991fc01cd02ce5616f0bb1cf0c85653dd42348))
* **security:** remediate Rook audit findings ([#370](https://github.com/dallay/cortex/issues/370)) ([2961bae](https://github.com/dallay/cortex/commit/2961bae6b0ea92eb861cda4342811e70cb462242))
* **security:** resolve open code scanning and Dependabot alerts ([#77](https://github.com/dallay/cortex/issues/77)) ([a4ad9d4](https://github.com/dallay/cortex/commit/a4ad9d4689e6dfc2c3852952644097c1388f6d37))
* **tests:** use std::env::temp_dir() for cross-platform temp DB path ([6147425](https://github.com/dallay/cortex/commit/6147425e55ca3e8e333437e82317df228d76a19b))


### Documentation

* improve all READMEs with professional design and badges ([#386](https://github.com/dallay/cortex/issues/386)) ([5271f51](https://github.com/dallay/cortex/commit/5271f510dc6da31bf99be4c00dfaf8bad92b187a))
* **readme:** convert badges to dynamic shields.io endpoints ([#397](https://github.com/dallay/cortex/issues/397)) ([a66ede5](https://github.com/dallay/cortex/commit/a66ede558d8ab794464f31b2d2919d5988206bb7))
* reorganize documentation structure ([#364](https://github.com/dallay/cortex/issues/364)) ([f3d7803](https://github.com/dallay/cortex/commit/f3d78031542b43bcf8391611a63068c87504bf69))


### Code Refactoring

* **dashboard:** move locale and theme selectors to header bar ([4b07441](https://github.com/dallay/cortex/commit/4b07441d695e8391c9f8bf26dfc52d2b9ec0bd0c))
* **dashboard:** remove unused imports and dead code from views ([#94](https://github.com/dallay/cortex/issues/94)) ([8a2b717](https://github.com/dallay/cortex/commit/8a2b7177968dbb51d5d17457061c4fdf28badc49))
* **providers:** use WireAuthType alias in AddProviderDialog helper ([01ffd1c](https://github.com/dallay/cortex/commit/01ffd1c06fb405a8352789b811e01a850e5ad5c8))
* reduce cognitive complexity in 8 functions to fix SonarCloud quality gate ([7a768b3](https://github.com/dallay/cortex/commit/7a768b3f41af5b280db69a33f6ae7fb48c660cdf))
* **specs:** rewrite provider-connections specs as technology-agnostic ([#2](https://github.com/dallay/cortex/issues/2)) ([7474a63](https://github.com/dallay/cortex/commit/7474a63cf137af55b98a2a00c1c7d18c761dc8d0))
* **tests:** extract shared NoOp fakes, dedup provider catalog and icon spec ([#293](https://github.com/dallay/cortex/issues/293)) ([79ccdb9](https://github.com/dallay/cortex/commit/79ccdb949d322e358b8e6cdf539429b8aad67994))


### Tests

* add model passing tests to di_tests ([121327c](https://github.com/dallay/cortex/commit/121327cfedbc355a2b9e0a8020051736f6822a3c))
* add unit tests for db-migration, transport-axum, and rook di ([#441](https://github.com/dallay/cortex/issues/441)) ([b952637](https://github.com/dallay/cortex/commit/b952637df55c167794da2fddbbc2f6ff850947e0))
* **cache:** add e2e integration tests and archive read-cache change ([#112](https://github.com/dallay/cortex/issues/112)) ([6e6f831](https://github.com/dallay/cortex/commit/6e6f8316de188c7b9d2d591215c4fd947a75c32b))
* improve coverage and fix flaky TLS test ([#382](https://github.com/dallay/cortex/issues/382)) ([c17c737](https://github.com/dallay/cortex/commit/c17c737c9ec2a983e4c01f5d8a4888b8491fcd6e))
* increase coverage on current main ([#373](https://github.com/dallay/cortex/issues/373)) ([f699a13](https://github.com/dallay/cortex/commit/f699a1341ad8cd4cc84a2ef77f23b1160f23234d))


### Chores

* agent MVP working tree snapshot ([#183](https://github.com/dallay/cortex/issues/183)) ([562a438](https://github.com/dallay/cortex/commit/562a4385f52ffbb8336fba1f4213bf75e7d28d14))
* code-quality hardening (dead code, lock poisoning, Anthropic stream fix, dashboard typecheck) ([#425](https://github.com/dallay/cortex/issues/425)) ([2178a45](https://github.com/dallay/cortex/commit/2178a4580b379aa7a8c5db76c786b4b4ef30795a))
* **dashboard:** use single root pnpm workspace source of truth ([#336](https://github.com/dallay/cortex/issues/336)) ([d106db9](https://github.com/dallay/cortex/commit/d106db96bcea4fd1f801e9eac84b3bbc7025b59d))
* **deps-dev:** bump undici from 5.29.0 to 6.29.1 in /apps/rook/dashboard in the npm_and_yarn group across 1 directory ([#384](https://github.com/dallay/cortex/issues/384)) ([139310b](https://github.com/dallay/cortex/commit/139310be824b533ab6a1e1b81f079a6ea594344e))
* **deps-docker:** update docker ([#512](https://github.com/dallay/cortex/issues/512)) ([98db52d](https://github.com/dallay/cortex/commit/98db52dc0ee1ced3009b39314c4d924f8efa244d))
* **deps-js:** update dependency @biomejs/biome to v2.6.3 ([#494](https://github.com/dallay/cortex/issues/494)) ([aa49db9](https://github.com/dallay/cortex/commit/aa49db947573f55b0682b4cc4bf4f7790eab43d7))
* **deps-js:** update dependency vitest to v4.1.11 [security] ([#184](https://github.com/dallay/cortex/issues/184)) ([7bb1005](https://github.com/dallay/cortex/commit/7bb1005f6849f833e83bb22b66711460a57ef43e))
* **deps-js:** update javascript dependencies ([#284](https://github.com/dallay/cortex/issues/284)) ([f829344](https://github.com/dallay/cortex/commit/f829344f76066ad2f54a0f76ad19eab877197f2c))
* **deps-js:** update javascript dependencies ([#300](https://github.com/dallay/cortex/issues/300)) ([e8cec9c](https://github.com/dallay/cortex/commit/e8cec9c53caea7504d3aec18c404ddcc985c110a))
* **deps-js:** update javascript dependencies ([#316](https://github.com/dallay/cortex/issues/316)) ([9fe0ac0](https://github.com/dallay/cortex/commit/9fe0ac01ebf72f47a2eb37a12f4d42d6c692240a))
* **deps-js:** update javascript dependencies ([#322](https://github.com/dallay/cortex/issues/322)) ([abc4433](https://github.com/dallay/cortex/commit/abc443311e42db600e811d5ab51a1225e5f7ae06))
* **deps-js:** update javascript dependencies ([#325](https://github.com/dallay/cortex/issues/325)) ([044ceb5](https://github.com/dallay/cortex/commit/044ceb5d8a5a0a8f56459fae7ef4992159988366))
* **deps-js:** update javascript dependencies ([#377](https://github.com/dallay/cortex/issues/377)) ([94ac5bc](https://github.com/dallay/cortex/commit/94ac5bc847d44ec323d8cb6ecfe724ee84c9dcde))
* **deps-js:** update javascript dependencies ([#389](https://github.com/dallay/cortex/issues/389)) ([e97c9f0](https://github.com/dallay/cortex/commit/e97c9f076aa7bdd1e2cbe66f66bf70ae65845f24))
* **deps-js:** update javascript dependencies ([#434](https://github.com/dallay/cortex/issues/434)) ([437df98](https://github.com/dallay/cortex/commit/437df983d27fbf97fea68ff299cda6e200f5c7c5))
* **deps-js:** update javascript major ([#391](https://github.com/dallay/cortex/issues/391)) ([a3a2a19](https://github.com/dallay/cortex/commit/a3a2a19b51d979bec6fa79081aaa4ccf28e4d657))
* **deps-rust:** bump dirs from 5.0.1 to 6.0.0 ([#14](https://github.com/dallay/cortex/issues/14)) ([36d59ac](https://github.com/dallay/cortex/commit/36d59ac80724e7ed0f9ee69af02a09eb98023191))
* **deps-rust:** bump toml from 0.8.23 to 1.1.2+spec-1.1.0 ([#7](https://github.com/dallay/cortex/issues/7)) ([b52a6d5](https://github.com/dallay/cortex/commit/b52a6d553a41ef256c5f1f9f91bfb686cd058408))
* **deps:** consolidate dependency updates (docker + actions + js pins) ([#369](https://github.com/dallay/cortex/issues/369)) ([de5f49f](https://github.com/dallay/cortex/commit/de5f49f7eec0404b0b03aadda0fe397fa7eb2370))
* **deps:** lock file maintenance ([#301](https://github.com/dallay/cortex/issues/301)) ([4c9db6a](https://github.com/dallay/cortex/commit/4c9db6af5d7dff1dc3a103abe00cbd489e7f874e))
* **deps:** update pnpm, markdownlint-cli2, vue-router, and devDependencies to latest versions ([bf3d969](https://github.com/dallay/cortex/commit/bf3d969b5418ab0dd329494a2a857b818da00590))
* enforce coverage gates and split large use cases ([#436](https://github.com/dallay/cortex/issues/436)) ([37c24e0](https://github.com/dallay/cortex/commit/37c24e0ba59b6e984723d0cf2da462ee9ecfd216))
* initial scaffold ([7da1a73](https://github.com/dallay/cortex/commit/7da1a7359d873c0b5d979e82b1f266bf453674d1))
* **skills:** harden Cortex agent guidance ([#519](https://github.com/dallay/cortex/issues/519)) ([df4d944](https://github.com/dallay/cortex/commit/df4d9441baf992d735824efed2b1d6af2ab513cd))
* **tests:** reformat assertions and function calls for improved readability ([d4e33ad](https://github.com/dallay/cortex/commit/d4e33ad929d1815e2489038f9d76d7cd541aa40d))

## [0.0.1] - Unreleased

- Initial product-scoped release metadata.
