<a name="1.49.4"></a>
## 1.49.4 (2018-08-01)


#### Bug Fixes

*   oops, check the user's actual month for validity ([8b4eb87a](https://github.com/mozilla-services/autopush-rs/commit/8b4eb87acad89de11c5218ae9dce7191c113c5cc), closes [#47](https://github.com/mozilla-services/autopush-rs/issues/47))



<a name="1.49.3"></a>
## 1.49.3 (2018-07-30)




<a name="1.49.2"></a>
## 1.49.2 (2018-07-27)


#### Bug Fixes

*   limit valid message tables to the last 3 ([69adfb4e](https://github.com/mozilla-services/autopush-rs/commit/69adfb4e434fd66ad53196e6eae2b6666d5cfaf8))



<a name="1.49.1"></a>
## 1.49.1 (2018-07-26)


#### Features

*   log out failed conversion items and use histogram for timers ([05a71d9b](https://github.com/mozilla-services/autopush-rs/commit/05a71d9b83592b2672cdd04fdb66829a64e0c95f))



<a name="1.49.0"></a>
## 1.49.0 (2018-07-20)


#### Refactor

*   push table names down into DynamoStorage ([750c00ff](https://github.com/mozilla-services/autopush-rs/commit/750c00ff8940f24509c8fed2c92364843fe06fc4))
*   split SendThenWait into 2 states ([155b9fc0](https://github.com/mozilla-services/autopush-rs/commit/155b9fc093f50195c56afb45966941921b03ec0a))

#### Features

*   emit metrics for any failed notif conversions ([4a50700f](https://github.com/mozilla-services/autopush-rs/commit/4a50700f9fb6ab28930f56c41f933230f7ff9e2c), closes [#33](https://github.com/mozilla-services/autopush-rs/issues/33))
*   log the nack code in metrics ([b795566d](https://github.com/mozilla-services/autopush-rs/commit/b795566db383884f005b69f2ad1e3d09d3de363c), closes [#34](https://github.com/mozilla-services/autopush-rs/issues/34))
*   add dockerflow requirements ([13c0fffc](https://github.com/mozilla-services/autopush-rs/commit/13c0fffc78fb9ef7a4b2bb72ced0b1fc87122138), closes [#29](https://github.com/mozilla-services/autopush-rs/issues/29))
*   update dependencies ([f7ded753](https://github.com/mozilla-services/autopush-rs/commit/f7ded753801b90c608fd8c2c54e255cb2b6c0241), closes [#25](https://github.com/mozilla-services/autopush-rs/issues/25))

#### Bug Fixes

*   typo and image link fix ([903e46d8](https://github.com/mozilla-services/autopush-rs/commit/903e46d813696632a6de0cf41d901af85edc9f6c))
*   drop users with too many stored messages ([86c65cae](https://github.com/mozilla-services/autopush-rs/commit/86c65cae962e82ac5d97ae7f266e8d31e48d9a50), closes [#25](https://github.com/mozilla-services/autopush-rs/issues/25))



<a name="1.48.2"></a>
## 1.48.2 (2018-07-10)


#### Features

*   setup a cadence error handler ([26bb9084](https://github.com/mozilla-services/autopush-rs/commit/26bb9084d9a2be6dac0c9fee6ba5fae8e2d3ca2d), closes [#3](https://github.com/mozilla-services/autopush-rs/issues/3))

#### Bug Fixes

*   stop notification fetch spinning ([72a85ebb](https://github.com/mozilla-services/autopush-rs/commit/72a85ebbfa4a505f0817cd252e43b0541fd0627d))
*   fix unset connected_at values ([8f81af35](https://github.com/mozilla-services/autopush-rs/commit/8f81af35020884e92da6cbc90ec5a0bd6af411ff), closes [#24](https://github.com/mozilla-services/autopush-rs/issues/24))

#### Refactor

*   some more renaming ([1d5e7188](https://github.com/mozilla-services/autopush-rs/commit/1d5e718849c19b882ec777c95f2c199cbb97851f), closes [#14](https://github.com/mozilla-services/autopush-rs/issues/14))
*   service -> broadcast ([cdfb1690](https://github.com/mozilla-services/autopush-rs/commit/cdfb169079d98cc0f2b76bed2a3eff0564a10ddc))



<a name="1.48.1"></a>
## 1.48.1 (2018-06-26)


#### Chore

*   disable default app user for now ([2b7d1a9e](https://github.com/mozilla-services/autopush-rs/commit/2b7d1a9eb0ae81267c6b002929202927abb03d2d))

#### Bug Fixes

*   fix hostname lookup not including a port ([b6f57cb8](https://github.com/mozilla-services/autopush-rs/commit/b6f57cb86a9b6e2bfc81b677bdb3406563263a55))
*   resolve intermittent monthly integration test fails ([3ee6614c](https://github.com/mozilla-services/autopush-rs/commit/3ee6614ccfa4423eebedd61f156b25d40fa0c37d))
*   detailed resolver errors ([6bb28548](https://github.com/mozilla-services/autopush-rs/commit/6bb28548be78a95780d9855ed4c056084f90562c))



<a name="1.48.0"></a>
## 1.48.0 (2018-06-22)


#### Chore

*   Dockerfile/ci fixes ([b4f3f912](https://github.com/mozilla-services/autopush-rs/commit/b4f3f9122cbba1434fcc0eb5a91f3e63dafd813d))

#### Bug Fixes

*   match python autopush's crypto_key format ([0eeabcbf](https://github.com/mozilla-services/autopush-rs/commit/0eeabcbf4205c37a7b5b48615497a25b4afb2a77), closes [#11](https://github.com/mozilla-services/autopush-rs/issues/11))

#### Features

*   transfer python integration tests and docker/circlci building ([60deca51](https://github.com/mozilla-services/autopush-rs/commit/60deca5172308cbce0bd4d41ec762b2f3caede64), closes [#1](https://github.com/mozilla-services/autopush-rs/issues/1))
*   initial transfer of Rust autopush code ([2e4818db](https://github.com/mozilla-services/autopush-rs/commit/2e4818db123035e26721201c32dd88e7bbf723ae))

#### Doc

*   update documentation ([1d244864](https://github.com/mozilla-services/autopush-rs/commit/1d24486497c5bad20d74c9d065b07a83b192523c))



