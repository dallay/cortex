# Changelog

All notable Rook releases are documented in this file.

The file is maintained by Release Please. Rook releases use the independent
`rook@vX.Y.Z` tag and never imply a repository-wide Cortex version.

## [0.1.0](https://github.com/dallay/cortex/compare/rook@v0.0.1...rook@v0.1.0) (2026-11-28)


### Features

* add dual-layer request deduplication with provider token caching ([#123](https://github.com/dallay/cortex/issues/123)) ([768e2d8](https://github.com/dallay/cortex/commit/768e2d883b0902644ff4f6b256f778a7ea6ae6f3))
* add provider quota tracking with upstream observations ([#524](https://github.com/dallay/cortex/issues/524)) ([ca1547b](https://github.com/dallay/cortex/commit/ca1547b2027a323fb598c259eb45fa67469f09c9))
* add secure OpenAI-compatible providers ([#421](https://github.com/dallay/cortex/issues/421)) ([16bdad9](https://github.com/dallay/cortex/commit/16bdad94832e67a4e68e37ec0baac1b58c13a595))
* add startup banners for rook and huginn ([#547](https://github.com/dallay/cortex/issues/547)) ([d69096c](https://github.com/dallay/cortex/commit/d69096c4864de50a9d3dc5809824bb3d345068e0))
* anthropic migration ([e6cd47a](https://github.com/dallay/cortex/commit/e6cd47a4c301aca9a1c1c069c636fc746140a5a2))
* **bazel:** introduce the Bazel bootstrap and an informational crate classifier ([#729](https://github.com/dallay/cortex/issues/729)) ([317e72a](https://github.com/dallay/cortex/commit/317e72aacbb761e47b42638f2b546bb71d1d0d4b))
* **dashboard:** add Codecov bundle analysis plugin ([#129](https://github.com/dallay/cortex/issues/129)) ([48ca9df](https://github.com/dallay/cortex/commit/48ca9dff75a88fe239621bc8202199e2f46197bd))
* **dashboard:** branded provider icons and external title links ([#139](https://github.com/dallay/cortex/issues/139)) ([775439b](https://github.com/dallay/cortex/commit/775439b074bc07aabb18a1b8cf334975569aa795))
* **dashboard:** complete admin API parity ([#437](https://github.com/dallay/cortex/issues/437)) ([2980e12](https://github.com/dallay/cortex/commit/2980e12ee828e6df9594c1dac899ea0abb39d978))
* **dashboard:** refactor providers UI to 3-screen catalog/details/connection hierarchy ([b1eb991](https://github.com/dallay/cortex/commit/b1eb9912a4cfd9df688becc5ce82be9b4d9b5d10))
* **dashboard:** render application name and version dynamically ([#339](https://github.com/dallay/cortex/issues/339)) ([205053e](https://github.com/dallay/cortex/commit/205053e98c73ebb6dd8623ad989689371ba4adb4))
* enable independent Rook and Huginn releases with Release Please ([#333](https://github.com/dallay/cortex/issues/333)) ([48e3536](https://github.com/dallay/cortex/commit/48e3536cf95f9c2ab765800c2c4fe2fe5ebb427e))
* harden Huginn boundaries and add Makefile workflow ([#338](https://github.com/dallay/cortex/issues/338)) ([80e352f](https://github.com/dallay/cortex/commit/80e352fd67459fa4e8a314852a3ef76ba3612bb6))
* **huginn:** add workspace file index ([#344](https://github.com/dallay/cortex/issues/344)) ([59d30dd](https://github.com/dallay/cortex/commit/59d30dd93cb00608c86b9243a29ebeddf6163a0e))
* **huginn:** redesign permission approvals ([#614](https://github.com/dallay/cortex/issues/614)) ([4ea0816](https://github.com/dallay/cortex/commit/4ea0816acabd07f9911c6f082a4391b9ee7a0933))
* integrate Cortex brand identity across dashboard and CLI apps ([#760](https://github.com/dallay/cortex/issues/760)) ([4697f6a](https://github.com/dallay/cortex/commit/4697f6abff02c80c6f35b9f624bdf865aabf9940))
* **muninn:** achieve functional parity with Agent Memory on Bolt ([#740](https://github.com/dallay/cortex/issues/740)) ([d8a5446](https://github.com/dallay/cortex/commit/d8a5446d1e2a72e57629cdf12514679f2c093fc0))
* **muninn:** implement persistent consolidation and harden summary isolation ([#771](https://github.com/dallay/cortex/issues/771)) ([d98ed9d](https://github.com/dallay/cortex/commit/d98ed9d3fa9aff828928e8959053897b8c2472e2))
* **oauth:** add provider token lifecycle ([#341](https://github.com/dallay/cortex/issues/341)) ([f596566](https://github.com/dallay/cortex/commit/f5965667d5158b0bdfb2b2e422e055cda3b5f10d))
* **ollama:** preservar metadatos de respuesta y llamadas nativas ([#319](https://github.com/dallay/cortex/issues/319)) ([018b464](https://github.com/dallay/cortex/commit/018b464debdddd6b9ddf7b6bb3364448d156c5e1))
* **providers-ollama:** 2-step health probe + hidden baseUrl for managed cloud ([f934acd](https://github.com/dallay/cortex/commit/f934acd3c4f853e723e412ec831d194ff8e7f81c))
* **providers:** add real quota dashboard ([a5d2f91](https://github.com/dallay/cortex/commit/a5d2f91f811d61b6389f14c8936cba37e2c3fd1d))
* **providers:** add real quota dashboard ([#155](https://github.com/dallay/cortex/issues/155)) ([d08c193](https://github.com/dallay/cortex/commit/d08c193740d1261dc82a50cf5bc2432b9df122fa))
* **providers:** add test-credentials endpoint to fix CONFLICT bug ([541f5ce](https://github.com/dallay/cortex/commit/541f5cef00d84486ffc7c66bdf95abcce73068f9))
* **providers:** add warning state to credential validation ([a411b50](https://github.com/dallay/cortex/commit/a411b50dbe7b96d7aae54888283595c2dd400667))
* **quota:** capture provider quota headers with persistence and HTTP wiring ([#330](https://github.com/dallay/cortex/issues/330)) ([0d2cc04](https://github.com/dallay/cortex/commit/0d2cc04b1914ce7a264f190ce2c6fabc2284b3a7))
* **rook:** add capability-aware model catalog ([#466](https://github.com/dallay/cortex/issues/466)) ([271bd42](https://github.com/dallay/cortex/commit/271bd4242f4a48523bf972574d6c13ef86ff16fb))
* **rook:** add truthful provider usage dashboard ([#371](https://github.com/dallay/cortex/issues/371)) ([26e719a](https://github.com/dallay/cortex/commit/26e719a91fcf3019e1f242ec96828da4a02e7af8))
* **rook:** drive provider catalog from backend ([#617](https://github.com/dallay/cortex/issues/617)) ([1b154aa](https://github.com/dallay/cortex/commit/1b154aa6a768a7532e42c770e789e050bdaa05f0))
* **rook:** enforce configurable per-API-key spending budgets (DALLAY-686) ([#517](https://github.com/dallay/cortex/issues/517)) ([e6e5397](https://github.com/dallay/cortex/commit/e6e5397c75b656652ea35459004464e06cb0cb79))
* **rook:** normalize model aliases before combo routing ([#324](https://github.com/dallay/cortex/issues/324)) ([a17a6e7](https://github.com/dallay/cortex/commit/a17a6e70639f401df85cf224a7b086aa77a290b9))
* **rook:** reorganize providers catalog with 7 categories ([#669](https://github.com/dallay/cortex/issues/669)) ([edc0dcc](https://github.com/dallay/cortex/commit/edc0dcccdcc3fbd554bcdb3555d705607bcaf59d))
* **rook:** replace RoutingStrategy with six explicit routing policies ([#664](https://github.com/dallay/cortex/issues/664)) ([639eec6](https://github.com/dallay/cortex/commit/639eec6fdb668ec7bececbace913f03463fa2fc9))
* **rook:** support direct PKCE OAuth account linking for Codex ([#775](https://github.com/dallay/cortex/issues/775)) ([7a913d5](https://github.com/dallay/cortex/commit/7a913d5d063d2e8cd2ceb1faa09ea479695ca25b))
* **rook:** virtual combo models, Anthropic tool-calling and gateway e2e ([#362](https://github.com/dallay/cortex/issues/362)) ([bc9b105](https://github.com/dallay/cortex/commit/bc9b105244fb01eead584cee8e77bc59fe9b3e33))
* unify Rook and Huginn home directories ([#492](https://github.com/dallay/cortex/issues/492)) ([dd5bbef](https://github.com/dallay/cortex/commit/dd5bbef7ea5cd2024b805bdeeb89332f3e638e14))
* update server port from 8080 to 3773 and adjust related configurations ([#147](https://github.com/dallay/cortex/issues/147)) ([eeb2798](https://github.com/dallay/cortex/commit/eeb27985bf290090351602c00d077a2208f56f8a))


### Bug Fixes

* address SonarCloud findings ([#398](https://github.com/dallay/cortex/issues/398)) ([77bd01c](https://github.com/dallay/cortex/commit/77bd01c0939870307193985601910901ca43cf6f))
* address the review findings of [#716](https://github.com/dallay/cortex/issues/716) ([#728](https://github.com/dallay/cortex/issues/728)) ([ca9df8c](https://github.com/dallay/cortex/commit/ca9df8c15b55fce86a11a18416a3d9be1ba68e70))
* address the review round on kernel docs and test assertions ([#567](https://github.com/dallay/cortex/issues/567)) ([ccfab1c](https://github.com/dallay/cortex/commit/ccfab1c48820210098335b9a8e905395fa1b3d0d))
* **bazel:** add missing rust_test targets to nine BUILD.bazel files ([d8a5446](https://github.com/dallay/cortex/commit/d8a5446d1e2a72e57629cdf12514679f2c093fc0))
* **ci:** allow whitespace in needs array for CI contract tests ([#686](https://github.com/dallay/cortex/issues/686)) ([9014ff9](https://github.com/dallay/cortex/commit/9014ff9f66eca8f91a0a53f5ee4a95b6ffbff8d4))
* **ci:** relax needs regex in CI contract tests and fix dashboard TypeScript errors ([#689](https://github.com/dallay/cortex/issues/689)) ([11c4f59](https://github.com/dallay/cortex/commit/11c4f59cb6fa637a6248117da1bc00e03a8c1117))
* **ci:** repair Bazel, release, and OSV workflows ([#750](https://github.com/dallay/cortex/issues/750)) ([53936a3](https://github.com/dallay/cortex/commit/53936a39aa04c9d88d7c7c472a596fecc9694aaf))
* **dashboard:** associate catalog search input with label ([#282](https://github.com/dallay/cortex/issues/282)) ([8c1dfa3](https://github.com/dallay/cortex/commit/8c1dfa35211d2aa1abf0b705b501c6ba8ed6e0a1))
* **dashboard:** associate labels with form controls ([#471](https://github.com/dallay/cortex/issues/471)) ([ce0d09e](https://github.com/dallay/cortex/commit/ce0d09ed3f33b6abe095d1b2e3a7c4df255acb43))
* **dashboard:** exclude .vscode from Biome scan scope ([#773](https://github.com/dallay/cortex/issues/773)) ([a99ba8e](https://github.com/dallay/cortex/commit/a99ba8e233b3488250ded9a11714a0a464fca736))
* **dashboard:** remove duplicate id on Select component ([#348](https://github.com/dallay/cortex/issues/348)) ([2996d27](https://github.com/dallay/cortex/commit/2996d274f8caa5c8e639b98929ef6512d9492f36))
* **deps-js:** pin dependencies ([#265](https://github.com/dallay/cortex/issues/265)) ([7547d33](https://github.com/dallay/cortex/commit/7547d33ebb93658a327336b79e295778449d749c))
* **deps-js:** update javascript major ([#380](https://github.com/dallay/cortex/issues/380)) ([b2f1cbb](https://github.com/dallay/cortex/commit/b2f1cbb15bff5bd3fefd633cbfdceeeefabec33a))
* **deps-rust:** update rust dependencies ([#378](https://github.com/dallay/cortex/issues/378)) ([1843cdf](https://github.com/dallay/cortex/commit/1843cdf55cebe24997709e330d94d415e6b1d738))
* **deps-rust:** update rust major ([#291](https://github.com/dallay/cortex/issues/291)) ([bb8cf2d](https://github.com/dallay/cortex/commit/bb8cf2dfccc657a7a5cca0af7dcaffb55d45dbcc))
* **deps:** restore trusted lefthook release ([#499](https://github.com/dallay/cortex/issues/499)) ([c2e3d1c](https://github.com/dallay/cortex/commit/c2e3d1c54e5d097186971aa51b0da0b4533e8f86))
* **e2e:** use getByRole('checkbox') for reka-ui Checkbox + cookie scope fix ([#145](https://github.com/dallay/cortex/issues/145)) ([aa062b2](https://github.com/dallay/cortex/commit/aa062b2f2f638238cddf9fca33f8339514e727ab))
* **huginn:** allow first-run without configured model ([#340](https://github.com/dallay/cortex/issues/340)) ([3be1190](https://github.com/dallay/cortex/commit/3be1190b31e1c52bd5b9ceea4ca096cb1c36918a))
* **muninn:** scope Bolt summary entities by owner ([#764](https://github.com/dallay/cortex/issues/764)) ([b18d520](https://github.com/dallay/cortex/commit/b18d520647cf9070a3e46fa2091974d428e94b51))
* populate provider supported_models from model catalog ([5a3ac0f](https://github.com/dallay/cortex/commit/5a3ac0f8321c8baf85d2dc537826f1bc81f36632))
* **providers:** link /providers/quota to tracking issue [#132](https://github.com/dallay/cortex/issues/132) ([54005ee](https://github.com/dallay/cortex/commit/54005ee06900ace1cdae71592cdcfe34b1902859))
* **providers:** restore es locale parity, tighten wire types, flatten component layout ([616d1d5](https://github.com/dallay/cortex/commit/616d1d5f685934480faf4742887d6773b8a51238))
* remediate CSRF token reuse, provider SSRF, and shell guard gaps ([#716](https://github.com/dallay/cortex/issues/716)) ([5255f3d](https://github.com/dallay/cortex/commit/5255f3da036f5a0da7c9f52d5f7b7f704526b161))
* resolve Sonar gate and Windows clippy ([#477](https://github.com/dallay/cortex/issues/477)) ([c1f28f1](https://github.com/dallay/cortex/commit/c1f28f15dd697eec4e4f709a34838dce205f2a86))
* resolve SonarCloud quality gate issues ([#304](https://github.com/dallay/cortex/issues/304)) ([e0dacca](https://github.com/dallay/cortex/commit/e0daccaefe21ca5a6c781c9dfc513a34a8430f91))
* **rook:** aplicar default_combo y advertir token_cache no soportado ([#465](https://github.com/dallay/cortex/issues/465)) ([3c611ba](https://github.com/dallay/cortex/commit/3c611bada8f80888358bd24c5704649fc8623fe6))
* **rook:** derive per-key budget cost bounds from catalog pricing ([#605](https://github.com/dallay/cortex/issues/605)) ([269ed5b](https://github.com/dallay/cortex/commit/269ed5b938ff9b43518cba7a6a682471672153d9))
* **rook:** resolve secret path aliases ([#468](https://github.com/dallay/cortex/issues/468)) ([548fec0](https://github.com/dallay/cortex/commit/548fec08b11ab624ec7887c432d40e0f09046184))
* **security:** address 9 findings from security audit ([#668](https://github.com/dallay/cortex/issues/668)) ([bdbca12](https://github.com/dallay/cortex/commit/bdbca122db73cf0ffd746d3a93319e00640fbe1c))
* **security:** auditar excepciones puntuales de rutas ([#478](https://github.com/dallay/cortex/issues/478)) ([565e633](https://github.com/dallay/cortex/commit/565e633706ba778179ed32e8de4511ea0be546cf))
* **security:** harden Rook rate limits and provider endpoints ([#349](https://github.com/dallay/cortex/issues/349)) ([fb4ebd6](https://github.com/dallay/cortex/commit/fb4ebd66797087331ded8729d1581e43bfe2189e))
* **security:** remediate code scanning findings ([#427](https://github.com/dallay/cortex/issues/427)) ([ec991fc](https://github.com/dallay/cortex/commit/ec991fc01cd02ce5616f0bb1cf0c85653dd42348))
* **security:** remediate Rook audit findings ([#370](https://github.com/dallay/cortex/issues/370)) ([2961bae](https://github.com/dallay/cortex/commit/2961bae6b0ea92eb861cda4342811e70cb462242))


### Performance

* **ci:** remove duplicated nextest run and pin llvm-cov provisioning ([#696](https://github.com/dallay/cortex/issues/696)) ([df6b8da](https://github.com/dallay/cortex/commit/df6b8da9f6fed8b7ad9c05472776598903bdb120))


### Documentation

* **development:** reorganize local development guides ([#613](https://github.com/dallay/cortex/issues/613)) ([99ca5fe](https://github.com/dallay/cortex/commit/99ca5fe5b03b5df0f42a36a8784ed67ca8827d19))
* improve all READMEs with professional design and badges ([#386](https://github.com/dallay/cortex/issues/386)) ([5271f51](https://github.com/dallay/cortex/commit/5271f510dc6da31bf99be4c00dfaf8bad92b187a))
* improve open source best practices and accessibility ([#692](https://github.com/dallay/cortex/issues/692)) ([410df38](https://github.com/dallay/cortex/commit/410df385daa6c44b1d48a2ea36c39dc4dc71533d))
* **readme:** convert badges to dynamic shields.io endpoints ([#397](https://github.com/dallay/cortex/issues/397)) ([a66ede5](https://github.com/dallay/cortex/commit/a66ede558d8ab794464f31b2d2919d5988206bb7))
* reorganize documentation structure ([#364](https://github.com/dallay/cortex/issues/364)) ([f3d7803](https://github.com/dallay/cortex/commit/f3d78031542b43bcf8391611a63068c87504bf69))
* **shared:** reconcile pre-GA security policy and actual CI controls ([#704](https://github.com/dallay/cortex/issues/704)) ([07789fd](https://github.com/dallay/cortex/commit/07789fd2bb2be29401d98492db928899d9e79f27))


### Styles

* format code and improve readability across multiple files ([63a3adf](https://github.com/dallay/cortex/commit/63a3adf152399dfb54b400a309a5b7a81466656c))
* format code for consistency and readability across multiple files ([836a0d7](https://github.com/dallay/cortex/commit/836a0d72ed347c42f35fce64730e95dbf9418074))


### Code Refactoring

* **providers:** use WireAuthType alias in AddProviderDialog helper ([01ffd1c](https://github.com/dallay/cortex/commit/01ffd1c06fb405a8352789b811e01a850e5ad5c8))
* **rook:** consolidate provider crates into four protocol adapters ([#612](https://github.com/dallay/cortex/issues/612)) ([095e724](https://github.com/dallay/cortex/commit/095e72467abf02fbe4387ebcb4bbf4bc70ee77ff))
* **rook:** delete the dead format-translation layer ([#597](https://github.com/dallay/cortex/issues/597)) ([931abea](https://github.com/dallay/cortex/commit/931abea6b991c386e07b06c56b6724a24a07a50c))
* **rook:** introduce the provider descriptor table and retrofit the drift sites (ROOK-P1-1/4/5) ([#610](https://github.com/dallay/cortex/issues/610)) ([8f198c9](https://github.com/dallay/cortex/commit/8f198c95eac708c3ebb8d838b58195bee5a99f90))
* **rook:** make the declared routing surface truthful ([#599](https://github.com/dallay/cortex/issues/599)) ([06ec463](https://github.com/dallay/cortex/commit/06ec463b813435b7e7f7c672f0be5b6d0dab0e01))
* **rook:** remove the unenforced tokens_per_minute limit dimension ([#603](https://github.com/dallay/cortex/issues/603)) ([35b748e](https://github.com/dallay/cortex/commit/35b748eec7f0aa52772409cab28d8b57dd027b8a))
* **tests:** extract shared NoOp fakes, dedup provider catalog and icon spec ([#293](https://github.com/dallay/cortex/issues/293)) ([79ccdb9](https://github.com/dallay/cortex/commit/79ccdb949d322e358b8e6cdf539429b8aad67994))


### Build System

* **bazel:** expand workspace target coverage ([#739](https://github.com/dallay/cortex/issues/739)) ([64f3ef1](https://github.com/dallay/cortex/commit/64f3ef171ed175abaae934c1e2fb51bc111e101b))
* **bazel:** make dashboard build hermetic with Vite 7 ([#745](https://github.com/dallay/cortex/issues/745)) ([5dcbd6f](https://github.com/dallay/cortex/commit/5dcbd6f0bda03faad95e872d3909d1781e61a95e))


### Continuous Integration

* **codecov:** add bundle and test analytics ([#643](https://github.com/dallay/cortex/issues/643)) ([3375291](https://github.com/dallay/cortex/commit/3375291c038ebb8645c7a2bf65c1048859cb409e))


### Tests

* add model passing tests to di_tests ([121327c](https://github.com/dallay/cortex/commit/121327cfedbc355a2b9e0a8020051736f6822a3c))
* add unit tests for db-migration, transport-axum, and rook di ([#441](https://github.com/dallay/cortex/issues/441)) ([b952637](https://github.com/dallay/cortex/commit/b952637df55c167794da2fddbbc2f6ff850947e0))
* improve coverage and fix flaky TLS test ([#382](https://github.com/dallay/cortex/issues/382)) ([c17c737](https://github.com/dallay/cortex/commit/c17c737c9ec2a983e4c01f5d8a4888b8491fcd6e))
* increase coverage on current main ([#373](https://github.com/dallay/cortex/issues/373)) ([f699a13](https://github.com/dallay/cortex/commit/f699a1341ad8cd4cc84a2ef77f23b1160f23234d))
* **rook:** prove NaN is descriptor-only ([#631](https://github.com/dallay/cortex/issues/631)) ([f7cb9a4](https://github.com/dallay/cortex/commit/f7cb9a4b30dd79c6c5528c75f72595a5b8211bea))


### Chores

* agent MVP working tree snapshot ([#183](https://github.com/dallay/cortex/issues/183)) ([562a438](https://github.com/dallay/cortex/commit/562a4385f52ffbb8336fba1f4213bf75e7d28d14))
* close the first Sonar slice (dashboard, lint, Dockerfile) ([#561](https://github.com/dallay/cortex/issues/561)) ([65a5e34](https://github.com/dallay/cortex/commit/65a5e3444352f3c609397915bcdfd2e759dc2897))
* code-quality hardening (dead code, lock poisoning, Anthropic stream fix, dashboard typecheck) ([#425](https://github.com/dallay/cortex/issues/425)) ([2178a45](https://github.com/dallay/cortex/commit/2178a4580b379aa7a8c5db76c786b4b4ef30795a))
* **dashboard:** use single root pnpm workspace source of truth ([#336](https://github.com/dallay/cortex/issues/336)) ([d106db9](https://github.com/dallay/cortex/commit/d106db96bcea4fd1f801e9eac84b3bbc7025b59d))
* **deps-dev:** bump undici from 5.29.0 to 6.29.1 in /apps/rook/dashboard in the npm_and_yarn group across 1 directory ([#384](https://github.com/dallay/cortex/issues/384)) ([139310b](https://github.com/dallay/cortex/commit/139310be824b533ab6a1e1b81f079a6ea594344e))
* **deps-docker:** update docker ([#512](https://github.com/dallay/cortex/issues/512)) ([98db52d](https://github.com/dallay/cortex/commit/98db52dc0ee1ced3009b39314c4d924f8efa244d))
* **deps-docker:** update docker ([#565](https://github.com/dallay/cortex/issues/565)) ([2b8b5af](https://github.com/dallay/cortex/commit/2b8b5af5579899b274c8bf3afcfcc1de104c1335))
* **deps-js:** update dependency @biomejs/biome to v2.6.3 ([#494](https://github.com/dallay/cortex/issues/494)) ([aa49db9](https://github.com/dallay/cortex/commit/aa49db947573f55b0682b4cc4bf4f7790eab43d7))
* **deps-js:** update dependency @biomejs/biome to v2.6.5 ([#619](https://github.com/dallay/cortex/issues/619)) ([b1716cc](https://github.com/dallay/cortex/commit/b1716ccc94bdc28d065dc30d9d811e701c381047))
* **deps-js:** update dependency @playwright/test to v1.65.0 ([#630](https://github.com/dallay/cortex/issues/630)) ([5c0a792](https://github.com/dallay/cortex/commit/5c0a792860990aeb07dd14fae79f142448e541b4))
* **deps-js:** update dependency jsdom to v31 ([#501](https://github.com/dallay/cortex/issues/501)) ([1577664](https://github.com/dallay/cortex/commit/157766442a59bf62569f00f2ceb7ea6ef9a707fa))
* **deps-js:** update dependency jsdom to v31.0.1 ([#670](https://github.com/dallay/cortex/issues/670)) ([dbbd630](https://github.com/dallay/cortex/commit/dbbd6305696c680f5973deb2a0091ff0e8f8ede1))
* **deps-js:** update dependency vitest to v4.1.11 [security] ([#184](https://github.com/dallay/cortex/issues/184)) ([7bb1005](https://github.com/dallay/cortex/commit/7bb1005f6849f833e83bb22b66711460a57ef43e))
* **deps-js:** update javascript dependencies ([#284](https://github.com/dallay/cortex/issues/284)) ([f829344](https://github.com/dallay/cortex/commit/f829344f76066ad2f54a0f76ad19eab877197f2c))
* **deps-js:** update javascript dependencies ([#300](https://github.com/dallay/cortex/issues/300)) ([e8cec9c](https://github.com/dallay/cortex/commit/e8cec9c53caea7504d3aec18c404ddcc985c110a))
* **deps-js:** update javascript dependencies ([#316](https://github.com/dallay/cortex/issues/316)) ([9fe0ac0](https://github.com/dallay/cortex/commit/9fe0ac01ebf72f47a2eb37a12f4d42d6c692240a))
* **deps-js:** update javascript dependencies ([#322](https://github.com/dallay/cortex/issues/322)) ([abc4433](https://github.com/dallay/cortex/commit/abc443311e42db600e811d5ab51a1225e5f7ae06))
* **deps-js:** update javascript dependencies ([#325](https://github.com/dallay/cortex/issues/325)) ([044ceb5](https://github.com/dallay/cortex/commit/044ceb5d8a5a0a8f56459fae7ef4992159988366))
* **deps-js:** update javascript dependencies ([#377](https://github.com/dallay/cortex/issues/377)) ([94ac5bc](https://github.com/dallay/cortex/commit/94ac5bc847d44ec323d8cb6ecfe724ee84c9dcde))
* **deps-js:** update javascript dependencies ([#389](https://github.com/dallay/cortex/issues/389)) ([e97c9f0](https://github.com/dallay/cortex/commit/e97c9f076aa7bdd1e2cbe66f66bf70ae65845f24))
* **deps-js:** update javascript dependencies ([#434](https://github.com/dallay/cortex/issues/434)) ([437df98](https://github.com/dallay/cortex/commit/437df983d27fbf97fea68ff299cda6e200f5c7c5))
* **deps-js:** update javascript dependencies ([#500](https://github.com/dallay/cortex/issues/500)) ([236185a](https://github.com/dallay/cortex/commit/236185a751f7b4e4753427a1a80ca1f19b4d241d))
* **deps-js:** update javascript dependencies ([#601](https://github.com/dallay/cortex/issues/601)) ([631805f](https://github.com/dallay/cortex/commit/631805f196d9edf09b4ecf178826ec3ea893b6dc))
* **deps-js:** update javascript dependencies ([#621](https://github.com/dallay/cortex/issues/621)) ([8ed87c4](https://github.com/dallay/cortex/commit/8ed87c4db7d6efef2fd0e991c7bcd82db074c2d3))
* **deps-js:** update javascript dependencies ([#633](https://github.com/dallay/cortex/issues/633)) ([f7907ef](https://github.com/dallay/cortex/commit/f7907ef6c250dbc5a4ba22f026e0e18700f2c0f1))
* **deps-js:** update javascript dependencies ([#665](https://github.com/dallay/cortex/issues/665)) ([113c650](https://github.com/dallay/cortex/commit/113c65090714cefa7262621dab6f5301c46e8a68))
* **deps-js:** update javascript dependencies ([#666](https://github.com/dallay/cortex/issues/666)) ([4a27211](https://github.com/dallay/cortex/commit/4a27211d929138288854c0658eb7622d3d0a846b))
* **deps-js:** update javascript dependencies ([#673](https://github.com/dallay/cortex/issues/673)) ([fc1698d](https://github.com/dallay/cortex/commit/fc1698df94b1bfd442c1a97feb497f91c673d513))
* **deps-js:** update javascript dependencies ([#717](https://github.com/dallay/cortex/issues/717)) ([5db13eb](https://github.com/dallay/cortex/commit/5db13ebe999cf60e1294ec6bde789642f71b14bb))
* **deps-js:** update javascript dependencies ([#733](https://github.com/dallay/cortex/issues/733)) ([c23160b](https://github.com/dallay/cortex/commit/c23160b8589a8e0447b719e81f2b481ba506e9f0))
* **deps-js:** update javascript major ([#391](https://github.com/dallay/cortex/issues/391)) ([a3a2a19](https://github.com/dallay/cortex/commit/a3a2a19b51d979bec6fa79081aaa4ccf28e4d657))
* **deps:** consolidate dependency updates (docker + actions + js pins) ([#369](https://github.com/dallay/cortex/issues/369)) ([de5f49f](https://github.com/dallay/cortex/commit/de5f49f7eec0404b0b03aadda0fe397fa7eb2370))
* **deps:** lock file maintenance ([#301](https://github.com/dallay/cortex/issues/301)) ([4c9db6a](https://github.com/dallay/cortex/commit/4c9db6af5d7dff1dc3a103abe00cbd489e7f874e))
* enforce coverage gates and split large use cases ([#436](https://github.com/dallay/cortex/issues/436)) ([37c24e0](https://github.com/dallay/cortex/commit/37c24e0ba59b6e984723d0cf2da462ee9ecfd216))
* fix all clippy and cargo warnings for clean main branch ([#671](https://github.com/dallay/cortex/issues/671)) ([e425bcb](https://github.com/dallay/cortex/commit/e425bcb7a2f8b282770a8c180c0ec44676ac62d1))
* merge origin/main into memory evaluation branch ([c52e08e](https://github.com/dallay/cortex/commit/c52e08e1686fae4e85dfbe032702da4a61f494f8))
* **skills:** harden Cortex agent guidance ([#519](https://github.com/dallay/cortex/issues/519)) ([df4d944](https://github.com/dallay/cortex/commit/df4d9441baf992d735824efed2b1d6af2ab513cd))
* update dependencies (frontend + backend) ([#649](https://github.com/dallay/cortex/issues/649)) ([e050d17](https://github.com/dallay/cortex/commit/e050d170599a1edd3e24b335fdae0e69d7c18409))

## [0.0.1] - Unreleased

- Initial product-scoped release metadata.
