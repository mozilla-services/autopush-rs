<a name="1.68.3"></a>
## 1.68.3 (2023-10-02)

#### Breaking Changes

*   Run GCM through FCMv1 HTTP API (#455) ([b2282277](https://github.com/mozilla-services/autopush-rs/commit/b2282277e7e751f5e513d65dda1ab2d2635e7299), breaks [#](https://github.com/mozilla-services/autopush-rs/issues/))

#### Features

*   Run GCM through FCMv1 HTTP API (#455) ([b2282277](https://github.com/mozilla-services/autopush-rs/commit/b2282277e7e751f5e513d65dda1ab2d2635e7299), breaks [#](https://github.com/mozilla-services/autopush-rs/issues/))

#### Bug Fixes

*   re-enable slog's envlogger (#452) ([e76b1198](https://github.com/mozilla-services/autopush-rs/commit/e76b11987400888bae17fdf61e7dff3edaa7743d))
*   AutopushUser fails with WebSocketConnectionClosedException ([5f8efb2d](https://github.com/mozilla-services/autopush-rs/commit/5f8efb2db9550b8960c6052c69e1103ab8681041))

#### Test

*   calibrate load tests ([1b5c5f99](https://github.com/mozilla-services/autopush-rs/commit/1b5c5f99a503ff5a70f009651ad4c47d39c6468e))
*   add subscribe and unsubscribe tasks to AutopushUser ([07d988dd](https://github.com/mozilla-services/autopush-rs/commit/07d988dd11a79d053eb93237f8de977490c5c510))


<a name="1.68.2"></a>
## 1.68.2 (2023-09-27)


#### Bug Fixes

*   Use `warn!()` to supplement sentry errors, since they don't show up regularly. (#449) ([4eaa9a06](https://github.com/mozilla-services/autopush-rs/commit/4eaa9a06a8197e31f103d9e0ce18f6aa550526b7))

#### Chore

*   tag 1.68.1 (#448) ([b55bbe76](https://github.com/mozilla-services/autopush-rs/commit/b55bbe7680be5e0601353321653fbefa9a9e737b))



>>>>>>> 68e447d2ac0c84550e438f86fa9d0e6fc20350d5
<a name="1.68.1"></a>
## 1.68.1 (2023-09-27)


#### Bug Fixes

*   don't emit backtraces in Display (#447) ([392f4e1e](https://github.com/mozilla-services/autopush-rs/commit/392f4e1e5c24cbb4d43fffaca5eb5697815f896c))

#### Chore

*   tag 1.68.0 (#446) ([00d19b1a](https://github.com/mozilla-services/autopush-rs/commit/00d19b1abb723fd47aa2d2fe9ab013fd153900f9))



<a name="1.68.0"></a>
## 1.68.0 (2023-09-27)


#### Features

*   emit a tag in autoconnect's metrics (#435) ([67186854](https://github.com/mozilla-services/autopush-rs/commit/67186854025a93c8868c202910431d1d645bbe82))
*   Add send notification task to locust load test file ([453ba8d4](https://github.com/mozilla-services/autopush-rs/commit/453ba8d4abf98bdf627c0b561ec6bf07ae68c5e8))
*   add an autoconnect-web Error type (#432) ([58086e77](https://github.com/mozilla-services/autopush-rs/commit/58086e77a7f0ae6d84a6a7bbce246b90cfadbad4))
*   switch the load tester to pypy-3.10 (#426) ([5b4d6d71](https://github.com/mozilla-services/autopush-rs/commit/5b4d6d71085480169a7f90202bb9295ad8b21f42))
*   Update loadtests user and tests ([d4af1bfe](https://github.com/mozilla-services/autopush-rs/commit/d4af1bfe22ccbc5df54812324553cc54b2c4f84a))
*   Add GCP BigTable support  (#364) ([608c52fe](https://github.com/mozilla-services/autopush-rs/commit/608c52fe6f905184ef0e0336b38812636295ab94))
*   consolidate the sentry middlwares into autopush_common ([e65486b9](https://github.com/mozilla-services/autopush-rs/commit/e65486b94ae6b381ee784b4caaebc9713fc78405))
*   add stacktraces to some sentry events (#406) ([0ded4de1](https://github.com/mozilla-services/autopush-rs/commit/0ded4de18458bbdeb3668cbaebc2582d9a4c942a))
*   Topic messages shouldn't have sortkey_timestamps (#402) ([eeff8d71](https://github.com/mozilla-services/autopush-rs/commit/eeff8d71a4167a497bd52e7c96c00023f9bb59e2))
*   build/deploy an autoconnect docker (#396) ([9ed4e6f0](https://github.com/mozilla-services/autopush-rs/commit/9ed4e6f01cfa0d7071bc78dbf62b8946e2ea55b8))
*   make DbClient's message table month optional (#393) ([ab3614b7](https://github.com/mozilla-services/autopush-rs/commit/ab3614b79854dbabeeef7d58e501aca36554c295))
*   remove legacy table rotation (#389) ([6aa107f5](https://github.com/mozilla-services/autopush-rs/commit/6aa107f522a08affc6d6dc8ad3753d03c313f48b))

#### Doc

*   fill in some autoconnect TODO docs (#410) ([74ffdb05](https://github.com/mozilla-services/autopush-rs/commit/74ffdb05d7b7943f0306eb821690a16d22df39fa))
*   Update docs for modern version of autopush (#388) ([d36fb527](https://github.com/mozilla-services/autopush-rs/commit/d36fb5275c1f5e2fb431da5d890e68f0698ab298))

#### Bug Fixes

*   Add better error messaging for GCM/FCM processing (#445) ([2e48f504](https://github.com/mozilla-services/autopush-rs/commit/2e48f504748211a0259b4049934dde7a99efdda6))
*   missing class-picker option in Kubernetes config ([9d0faf7c](https://github.com/mozilla-services/autopush-rs/commit/9d0faf7c2ddc3bbe7ef588987c3f526b5ed20e04))
*   apply fixes from code review 2 ([7ea158d2](https://github.com/mozilla-services/autopush-rs/commit/7ea158d26016f3835e989a42ba903c6aadb2ba46))
*   apply fixes from code review ([142c4d2c](https://github.com/mozilla-services/autopush-rs/commit/142c4d2c8cf10130eae6a7774743295bb28da66b))
*   load test docker build error in GCP ([e7e7539e](https://github.com/mozilla-services/autopush-rs/commit/e7e7539e97ec475b3c9c9ff59eb9be640d16f332))
*   remove print statement ([2134d050](https://github.com/mozilla-services/autopush-rs/commit/2134d050ac8b95ec516d1c580e926f96203aeb69))
*   load test script modifies kubernetes config on first run only ([89dc80bc](https://github.com/mozilla-services/autopush-rs/commit/89dc80bc5dce70c3e4ff67f4718672c0cc6eb325))
*   use explicit path ([a51fca5c](https://github.com/mozilla-services/autopush-rs/commit/a51fca5c90c20cf46887825ec23b537db1a7e35d))
*   remove mozsvc-common (#394) ([814ff49e](https://github.com/mozilla-services/autopush-rs/commit/814ff49eeae2c1d69cf30cd30789b157a6bcd784))
*   remove mozsvc-common (#394) ([66bb74f9](https://github.com/mozilla-services/autopush-rs/commit/66bb74f93f87d75433078a51158b1462997e3707))

#### Test

*   validate message schema in load tests with models ([0d8c1c2f](https://github.com/mozilla-services/autopush-rs/commit/0d8c1c2f707b1c34006abc43d41c79bc82e65c13))
*   add a load test shape ([f84e2665](https://github.com/mozilla-services/autopush-rs/commit/f84e2665003b894101a118a0b87ed0ac49ef770b))
*   calibrate load tests ([7a6c099f](https://github.com/mozilla-services/autopush-rs/commit/7a6c099f48d436a900495899c717b319a1d72523))

#### Chore

*   update user, spawn rate and time in load tests ([b9a88e5a](https://github.com/mozilla-services/autopush-rs/commit/b9a88e5ac907a879497521c03ba6800b7280e126))
*   add python linters and formatters to CI ([615b93ad](https://github.com/mozilla-services/autopush-rs/commit/615b93adf303d160ec5660600b8a703c1a15c421))
* **deps:**
  *  bump gevent from 23.9.0.post1 to 23.9.1 in /tests/load ([34eb330e](https://github.com/mozilla-services/autopush-rs/commit/34eb330e130288ce436b2e714140fac60a8627f8))
  *  bump cryptography from 40.0.2 to 41.0.4 in /tests ([acd35f27](https://github.com/mozilla-services/autopush-rs/commit/acd35f271e7c5fd3d5d9d7b6c85f121151f837e1))



<a name="1.67.3"></a>
## 1.67.3 (2023-06-01)


#### Bug Fixes

*   emit metrics also in the common Response error case (#384) ([5bd09339](https://github.com/mozilla-services/autopush-rs/commit/5bd0933997792bea824a9791e0fc91cc2fdf44be))



<a name="1.67.2"></a>
## 1.67.2 (2023-05-30)


#### Chore

* **deps:**  bump requests from 2.30.0 to 2.31.0 in /tests (#380) ([cdf91df6](https://github.com/mozilla-services/autopush-rs/commit/cdf91df6df7e197a7ced3ac1310b17291541f9b0))

#### Features

*   switch to latest release a2 library (#362) ([728fe169](https://github.com/mozilla-services/autopush-rs/commit/728fe169f2bd1f52100b65f744c1bfff3cbdd0fe))
*   add broadcast (megaphone) support (#381) ([97d3a3ae](https://github.com/mozilla-services/autopush-rs/commit/97d3a3aeff582de0f5cffa5affc431e6eac914c2))
*   complete (mostly) the WebPushClient (#379) ([f7110214](https://github.com/mozilla-services/autopush-rs/commit/f7110214d60cb77824b493d54035bd3ed65488ba))
*   move tests to python3 ([08bd46b8](https://github.com/mozilla-services/autopush-rs/commit/08bd46b896df625422f56a15fc9793295d0a084b))



<a name="1.67.1"></a>
## 1.67.1 (2023-05-05)


#### Bug Fixes

*   disable sentry's debug-images feature (#375) ([ed730974](https://github.com/mozilla-services/autopush-rs/commit/ed73097471c914ed555de6cc037b4a3db00e4974))



<a name="1.67.0"></a>
## 1.67.0 (2023-05-02)


#### Refactor

*   more db_client -> db ([61beb2e1](https://github.com/mozilla-services/autopush-rs/commit/61beb2e10f53253463ec5c20bf1ca504550707be))
*   options -> app_state ([5a6a35a5](https://github.com/mozilla-services/autopush-rs/commit/5a6a35a5759d827d7c89ca4b05c93b9cc6c8942f))
*   reduce settings duplication w/ deserialize_with ([4f3e4501](https://github.com/mozilla-services/autopush-rs/commit/4f3e4501d5755f4888002cd9a7250c257c70afdc))
*   move broadcast/protocol/registry into a common crate (#357) ([fa9109dc](https://github.com/mozilla-services/autopush-rs/commit/fa9109dc155676c1dd2231347d1513d93502d790))

#### Features

*   metric message table rotations (#371) ([647ffb14](https://github.com/mozilla-services/autopush-rs/commit/647ffb1496d8aacf43622a48e44351a73ad439dd))
*   quiet more router errors from sentry (#368) ([f90fc066](https://github.com/mozilla-services/autopush-rs/commit/f90fc066e0d6f79d0d070c406c30a9597736b3f0))
*   add the initial autoconnect-ws/state machine crates (#363) ([b4298eab](https://github.com/mozilla-services/autopush-rs/commit/b4298eab64a35439f03c07e4b88b58bc97c2fcd4))
*   reduce the number of errors reported to sentry.io (#358) ([a9a88f34](https://github.com/mozilla-services/autopush-rs/commit/a9a88f34147d29f08a1de3bd01545a715918d0e8))
*   Add metrics to try and analyze UpdateItem bug. (#344) ([7641d18d](https://github.com/mozilla-services/autopush-rs/commit/7641d18d9bfe53311099b2130302668dd7d2e113))
*   Add extra to sentry. [CONSVC-1886] (#333) ([008e3e8c](https://github.com/mozilla-services/autopush-rs/commit/008e3e8c400d85ab7b863e96d8d12a849733c8e9))

#### Bug Fixes

*   don't eat WebpushSocket poll_complete errors (#374) ([f8e65255](https://github.com/mozilla-services/autopush-rs/commit/f8e65255ba495d3115310ac5051ca47744de8ddb))
*   Add back missing metrics (#372) ([19525593](https://github.com/mozilla-services/autopush-rs/commit/19525593756c7b2573368a3a8bb0872d6207f04e))
*   make CORS default less restrictive. (#348) ([d421e8de](https://github.com/mozilla-services/autopush-rs/commit/d421e8de422b36f6e5838c3b46a99ab017ea08f0))
*   channel_id should be hyphenated (#365) ([0067ae42](https://github.com/mozilla-services/autopush-rs/commit/0067ae42f668bbf0afc3fc61fdd8c03364054053))



<a name="1.66.0"></a>
## 1.66.0 (2023-01-23)

#### Chore

*    release 1.65.0 (#324) ([060a520](https://github.com/mozilla-services/autopush-rs/commit/060a520f4acc05a82b29354ebfe17a1dad3b8044))

#### Bug Fixes

*    silence data overflow error in sentry (#327) ([202fba6](https://github.com/mozilla-services/autopush-rs/commit/202fba6e031751958f2c8b5dee3d81f32e49699c))

#### Features

*   add metrics for vapid errors (#340) ([922fcf8d](https://github.com/mozilla-services/autopush-rs/commit/922fcf8d50d5dd670f7d35a0056da9ffd165edcb))
*   add timeouts for Client::reqwest calls [CONSVC-3289] (#329) ([e0c370ae](https://github.com/mozilla-services/autopush-rs/commit/e0c370ae7d00405adf4f96ada9e94afe3208b76b))
*   modernize to rust 1.65 (#337) ([07b67bd](https://github.com/mozilla-services/autopush-rs/commit/07b67bda22e6b48195afd9ab16211d28fe46ec61))
*   add additional metrics for message tracking (#330) ([65ac1a3](https://github.com/mozilla-services/autopush-rs/commit/65ac1a3862b900133d9f7a80bd8dfd54b247292a))

#### Doc

*   Add mobile debugging steps to README (#339) ([9238056](https://github.com/mozilla-services/autopush-rs/commit/92380560bc15e097b18dc9142c73307f46430fbd))
*   Fix typo in README.md file (thanks @dev-aniketj) (#341)

<a name="1.65.0"></a>
## 1.65.0 (2022-07-20)


#### Features

*   allow for standard base64 private keys (#323) ([7ec9e541](https://github.com/mozilla-services/autopush-rs/commit/7ec9e5410db12ff9f17f97b3eb3da0f06f6d6c14))

#### Chore

*   tag 1.64.0 (#322) ([3b888782](https://github.com/mozilla-services/autopush-rs/commit/3b88878285a0efd06a560d3ecc5b97b705c93105))



<a name="1.64.0"></a>
##  (2022-07-13)


#### Bug Fixes

*   add jitter to retry (#319) ([3272fdec](https://github.com/mozilla-services/autopush-rs/commit/3272fdec1ccd144b0fdff678c64eddf27d45626f))
*   various mini-patches for FxA integration work (#321) ([b2b6bfd3](https://github.com/mozilla-services/autopush-rs/commit/b2b6bfd3e5f4273e6312f6305fb122013182d55b))
    * Added more verbose `trace!` and `debug!` logging messages.
    * ignore padding errors for VAPID keys
    * bumped up default max bytes to handle base64 encoded 4096 block
    * record the VapidError as an info before we send it to metrics

#### Chore

*   tag 1.63.0 (#312) ([f40a14a7](https://github.com/mozilla-services/autopush-rs/commit/f40a14a7972f19702d11075d7f49c6f29853b6c2))

#### Breaking Changes

*   Update for Jun 2022: Alters env var key names (#313) ([1ec85899](https://github.com/mozilla-services/autopush-rs/commit/1ec858990dabeefea5953b486dbd9beeada29ca2))
    Broke: Environment var key changes from:

    `AUTOPUSH_` => `AUTOPUSH__`

    `AUTOEND_` => `AUTOEND__`

<a name="1.63.0"></a>
## 1.63.0 (2022-06-02)


#### Chore

*   tag 1.62.0 (#304) ([1425b896](https://github.com/mozilla-services/autopush-rs/commit/1425b89641db05a44600fbcc01723a9c6f8e5f6f))

#### Bug Fixes

*   Fix GCM handling (#309) ([96cef485](https://github.com/mozilla-services/autopush-rs/commit/96cef485390118c8237f0738cb725856dfa3559e))

#### Features

*   Add tool to generate endpoints (#307) ([2829fa42](https://github.com/mozilla-services/autopush-rs/commit/2829fa42bfeffec1c8d2cdddd367a5555a52630b))



<a name="1.62.0"></a>
## 1.62.0 (2022-05-05)

##### Bug Fixes:

 *  bug: add app_id to error message, Add GCM data sends ([d2cd2ee5](https://github.com/mozilla-services/autopush-rs/commit/d2cd2ee52204ed4924b305ed651744a1cab2ebe9), closes [#303](https://github.com/mozilla-services/autopush-rs/issues/303)))


<a name="1.61.0"></a>
## 1.61.0 (2022-03-22)


#### Features

*   add status code and errno to reported metric for bridged errors (#291) ([772f020b](https://github.com/mozilla-services/autopush-rs/commit/772f020b844cc390884a12454931e028546cc826), closes [#288](https://github.com/mozilla-services/autopush-rs/issues/288))

#### Chore

*   tag 1.60.0 (#300) ([ef47c7a7](https://github.com/mozilla-services/autopush-rs/commit/ef47c7a7a0e6145a7bd325fcbe7204194612263a))



<a name="1.60.0"></a>
## 1.60.0 (2022-03-22)

#### Bug Fixes

*    bug: Do not report non-actionable errors to sentry (#299) ([3d18b10d2](https://github.com/mozilla-services/autopush-rs/commit/3d18b10d2e05a768f4cb1c2fa23f86b8df825a87))

#### Features

*   feat: return more explicit VAPID error message (#299) ([3d18b10d2](https://github.com/mozilla-services/autopush-rs/commit/3d18b10d2e05a768f4cb1c2fa23f86b8df825a87))


#### Chore

*   1.59.1 (#297) ([01d39582](https://github.com/mozilla-services/autopush-rs/commit/01d395826c1ab7359dea5b5405e2b1ffdd5e22df))



<a name="1.59.1"></a>
## 1.59.1 (2022-02-25)


#### Chore

*   tag 1.59 (#296) ([8b8e5fb3](https://github.com/mozilla-services/autopush-rs/commit/8b8e5fb36eb8909341af8c12e11ded6fc724d3c6))



<a name="1.59.0"></a>
## 1.59.0 (2022-02-25)


#### Chore

*   cleanup cargo audit invocation (#290) ([23818662](https://github.com/mozilla-services/autopush-rs/commit/23818662e17cf246fe96f17c2cfb41671b1e0015))
*   tag 1.58.0 (#289) ([e47f697b](https://github.com/mozilla-services/autopush-rs/commit/e47f697b29516acc7db53ecee708e41d4d26891b))
*   Q3 dependency update (#286) ([627456d9](https://github.com/mozilla-services/autopush-rs/commit/627456d9472971419bc4a8189b53d5b85e48ab30), closes [#285](https://github.com/mozilla-services/autopush-rs/issues/285))
*   Dep update for Jul 2021 (#283) ([230fb191](https://github.com/mozilla-services/autopush-rs/commit/230fb191d39bae2838b819362566fe5892394a4e))
*   library update for Jun 2021 (#280) ([5e9aabfe](https://github.com/mozilla-services/autopush-rs/commit/5e9aabfe54e4b2b0d0e0c8fa5d45f4e735f6e84f))
*   tag 1.57.8 (#277) ([35f0b406](https://github.com/mozilla-services/autopush-rs/commit/35f0b4066ef7873cf1a5c5d3c7f55bd4fb8d6387))

#### Bug Fixes

*   remove dbg!() (#292) ([1eca808f](https://github.com/mozilla-services/autopush-rs/commit/1eca808fc3ad641227b46d9e1ed52c8bf988097b), closes [#287](https://github.com/mozilla-services/autopush-rs/issues/287))

#### Features

*   Report bridge status via `__heartbeat__` (#295) ([4e85e401](https://github.com/mozilla-services/autopush-rs/commit/4e85e401df48c6c2057db2b82540f9009a90272a), closes [#294](https://github.com/mozilla-services/autopush-rs/issues/294))
*   Standardize to `/__error__` for sentry error check (#278) ([3f0dc8f4](https://github.com/mozilla-services/autopush-rs/commit/3f0dc8f43b42f49cf3108d34c3f5b901e3075e54), closes [#274](https://github.com/mozilla-services/autopush-rs/issues/274))



<a name="1.58.0"></a>
## 1.58.0 (2021-10-13)


#### Bug Fixes

*   fix local Authorization header (#284) ([0027d78f](https://github.com/mozilla-services/autopush-rs/commit/0027d78f76e9d5644ab95a0c3649555bc007a56c), closes [#282](https://github.com/mozilla-services/autopush-rs/issues/282))

#### Features

*   Standardize to `/__error__` for sentry error check (#278) ([3f0dc8f4](https://github.com/mozilla-services/autopush-rs/commit/3f0dc8f43b42f49cf3108d34c3f5b901e3075e54), closes [#274](https://github.com/mozilla-services/autopush-rs/issues/274))

#### Chore

*   Dep update for Jul 2021 (#283) ([230fb191](https://github.com/mozilla-services/autopush-rs/commit/230fb191d39bae2838b819362566fe5892394a4e))
*   library update for Jun 2021 (#280) ([5e9aabfe](https://github.com/mozilla-services/autopush-rs/commit/5e9aabfe54e4b2b0d0e0c8fa5d45f4e735f6e84f))



<a name="1.57.8"></a>
## 1.57.8 (2021-06-02)


#### Bug Fixes

*   use sentry 0.19 to match our backend server  (#276) ([44c85c0c](https://github.com/mozilla-services/autopush-rs/commit/44c85c0c6b6f093bebc4ca3383df7750140db236), closes [#275](https://github.com/mozilla-services/autopush-rs/issues/275))

#### Features

*   Drop aesgcm128 support (#268) ([d8b7ca83](https://github.com/mozilla-services/autopush-rs/commit/d8b7ca83542e2f0acf6dc6a3b277efe60aef276b))

#### Chore

*   tag 1.57.7 (#271) ([4857f0b1](https://github.com/mozilla-services/autopush-rs/commit/4857f0b1ea4e74ae4fce0dc411e1e4ba9758b26c))



<a name="1.57.7"></a>
## 1.57.7 (2021-03-26)


#### Chore

*   tag 1.57.6 ([802bfdfe](https://github.com/mozilla-services/autopush-rs/commit/802bfdfef56b394569ca571de3871c2aa59f44a6))

#### Bug Fixes

*   Add explicit `endpoint_url` setting (#270) ([5649d966](https://github.com/mozilla-services/autopush-rs/commit/5649d966e9eade5526efe240c52a84724d3a1020), closes [#269](https://github.com/mozilla-services/autopush-rs/issues/269))



<a name="1.57.6"></a>
## 1.57.6 (2021-03-16)

#### Bug

*   always return status 201 for success ([267213b8](https://github.com/mozilla-services/autopush-rs/pull/260/commits/123f970430e1b01ad6f31765be41ebbb267213b8), closes [#259](https://github.com/mozilla-services/autopush-rs/issues/259))


#### Features

*   convert FCM credential to string parameter ([5ba885af](https://github.com/mozilla-services/autopush-rs/commit/5ba885af5f2b68548047e2f83da36e364de7fbcb), closes [#254](https://github.com/mozilla-services/autopush-rs/issues/254))

#### Chore

*   tag 1.57.5 (#258) ([a95b7b97](https://github.com/mozilla-services/autopush-rs/commit/a95b7b979e5c75b2be80aa399b3bd36caa3156ab))
*   update dependencies for Mar 2021 ([7ec16f08](https://github.com/mozilla-services/autopush-rs/commit/7ec16f0877c1c6de5b08e429e1321536ae8ff9d8), closes [#256](https://github.com/mozilla-services/autopush-rs/issues/256))
*   tag 1.57.4 (#253) ([b4c5e5e3](https://github.com/mozilla-services/autopush-rs/commit/b4c5e5e38ecf9d9b4d06320a0daa457b2d280359))



<a name="1.57.5"></a>
## 1.57.5 (2021-03-01)


#### Chore

*   update dependencies for Mar 2021 ([7ec16f08](https://github.com/mozilla-services/autopush-rs/commit/7ec16f0877c1c6de5b08e429e1321536ae8ff9d8), closes [#256](https://github.com/mozilla-services/autopush-rs/issues/256))
*   tag 1.57.4 (#253) ([b4c5e5e3](https://github.com/mozilla-services/autopush-rs/commit/b4c5e5e38ecf9d9b4d06320a0daa457b2d280359))

#### Features

*   convert FCM credential to string parameter ([5ba885af](https://github.com/mozilla-services/autopush-rs/commit/5ba885af5f2b68548047e2f83da36e364de7fbcb), closes [#254](https://github.com/mozilla-services/autopush-rs/issues/254))



<a name="1.57.4"></a>
## 1.57.4 (2021-02-23)


#### Chore

*   Set baseline hyper version ([8bb5998c](https://github.com/mozilla-services/autopush-rs/commit/8bb5998c63186a86182af0912fcb6e11847d30cc), closes [#251](https://github.com/mozilla-services/autopush-rs/issues/251))
*   Update dependencies where possible. (#250) ([d9446f63](https://github.com/mozilla-services/autopush-rs/commit/d9446f630583f015c77e3d808fc0b6bb4ad4598a))
*   move a2 under mozilla-services (#245) ([1300a4dc](https://github.com/mozilla-services/autopush-rs/commit/1300a4dcce7f8390cfbc33f427e5fbebd318b00e), closes [#236](https://github.com/mozilla-services/autopush-rs/issues/236))
*   release/1.57.3 (#243) ([9bc9fef0](https://github.com/mozilla-services/autopush-rs/commit/9bc9fef001963b24ec13d9e2ec6e2485e5c04cea))



<a name="1.57.3"></a>
## 1.57.3 (2020-12-02)


#### Bug Fixes

*   Accept either paths or strings containing the cert for APNS (#241) ([b3dd8a3e](https://github.com/mozilla-services/autopush-rs/commit/b3dd8a3ee3a47424e13a4661337aa8f4a8106612), closes [#240](https://github.com/mozilla-services/autopush-rs/issues/240))

#### Chore

*   tag 1.57.1 (#239) ([0f168c93](https://github.com/mozilla-services/autopush-rs/commit/0f168c93c9758b515732231e0cae0f7a8a6779bb))



<a name="1.57.1"></a>
## 1.57.1 (2020-11-19)


#### Bug Fixes

*   Allow JSON formatted Auth keys ([d177a759](https://github.com/mozilla-services/autopush-rs/commit/d177a759b7550fcdb582d5463fc9f36b1838ffe0), closes [#234](https://github.com/mozilla-services/autopush-rs/issues/234))

#### Chore

*   release 1.57.0 (#231) ([761d91f4](https://github.com/mozilla-services/autopush-rs/commit/761d91f4fc06176dbc1ddc0d654d2b1981eebe81))
*   update circleci to use new docker auth ([fabe4954](https://github.com/mozilla-services/autopush-rs/commit/fabe4954c64944cf94d2b36bea5bcac89671bbbf))



<a name="1.57.0"></a>
## 1.57.0 (2020-10-16)


#### Features

*   Include minimal debug info in release builds (#215) ([fd659e3e](https://github.com/mozilla-services/autopush-rs/commit/fd659e3e613114ead60fc5b28348711bc57514ac), closes [#77](https://github.com/mozilla-services/autopush-rs/issues/77))
*   Support "gcm" as an alias to "fcm" (#211) ([fd0d63d2](https://github.com/mozilla-services/autopush-rs/commit/fd0d63d2767e85e76955eab232ca49f663bf7f60), closes [#204](https://github.com/mozilla-services/autopush-rs/issues/204))
*   Add the log-check route and remove unused ApiError variants/impls (#209) ([1b0b18b3](https://github.com/mozilla-services/autopush-rs/commit/1b0b18b3b37505acedbe32bf6a8fce325986aaa1), closes [#208](https://github.com/mozilla-services/autopush-rs/issues/208))
*   Amazon Device Messaging router (#207) ([c587446a](https://github.com/mozilla-services/autopush-rs/commit/c587446a96178a2359dcb7a3dc0bd5fb33947946), closes [#165](https://github.com/mozilla-services/autopush-rs/issues/165))
*   Use autoendpoint-rs in integration tests (#205) ([31d2d19c](https://github.com/mozilla-services/autopush-rs/commit/31d2d19c971ab2a90dfe47a62b17b422aaeec33a), closes [#168](https://github.com/mozilla-services/autopush-rs/issues/168))
*   Add the unregister user route (#195) ([b4bb1636](https://github.com/mozilla-services/autopush-rs/commit/b4bb163607c302e07f779486bfb0b8ef0b7da75c), closes [#179](https://github.com/mozilla-services/autopush-rs/issues/179))
*   APNS Router (#201) ([ce51957f](https://github.com/mozilla-services/autopush-rs/commit/ce51957f0dcb1a3ead8dab3d013e43f8e976064a), closes [#164](https://github.com/mozilla-services/autopush-rs/issues/164))
*   New channel endpoint (#189) ([6cc9a7dc](https://github.com/mozilla-services/autopush-rs/commit/6cc9a7dcae95e6978cfe15c003db199ebba6fd85))
*   Update token endpoint (#188) ([bb395fb2](https://github.com/mozilla-services/autopush-rs/commit/bb395fb2bfa620016baff174ac14d57786a4197c), closes [#177](https://github.com/mozilla-services/autopush-rs/issues/177))
*   Sentry integration for autoendpoint (#196) ([674d7d2c](https://github.com/mozilla-services/autopush-rs/commit/674d7d2c1ec7d79e41e871fc0fb39dc613353e32), closes [#155](https://github.com/mozilla-services/autopush-rs/issues/155))
*   Delete message endpoint (#186) ([6a7fa492](https://github.com/mozilla-services/autopush-rs/commit/6a7fa49209bbe576840d3c40f641e854257e4417), closes [#175](https://github.com/mozilla-services/autopush-rs/issues/175))
*   User registration (#185) ([6df3e36b](https://github.com/mozilla-services/autopush-rs/commit/6df3e36bab07a68d04e7bdfff23ed327d6f98e6b), closes [#176](https://github.com/mozilla-services/autopush-rs/issues/176))
*   Route notifications to FCM (Android) (#171) ([d9a0d9d7](https://github.com/mozilla-services/autopush-rs/commit/d9a0d9d7d1bd79fc3cd786e4a27a18bca4ff3eec), closes [#162](https://github.com/mozilla-services/autopush-rs/issues/162))
*   Route notifications to autopush connection servers (#167) ([e73dff17](https://github.com/mozilla-services/autopush-rs/commit/e73dff17e9a9909743fd57bbd9d6769789a455e1), closes [#161](https://github.com/mozilla-services/autopush-rs/issues/161))
*   Return detailed autoendpoint errors (#170) ([91d483ab](https://github.com/mozilla-services/autopush-rs/commit/91d483ab5e357ad6c50dc7669a9d12cd3f1914a7), closes [#159](https://github.com/mozilla-services/autopush-rs/issues/159))
*   Record the encoding in a metric if there is an encrypted payload (#166) ([8451d3f9](https://github.com/mozilla-services/autopush-rs/commit/8451d3f9790a6633107cd5473768c1dd53602df1))
*   Validate autoendpoint JWT tokens (#154) ([04fee7f9](https://github.com/mozilla-services/autopush-rs/commit/04fee7f9cbb7fc984d34953c05f34f7a732310d4), closes [#103](https://github.com/mozilla-services/autopush-rs/issues/103))
*   Validate user subscription data in autoendpoint (#160) ([8efa42c8](https://github.com/mozilla-services/autopush-rs/commit/8efa42c8885865ae1392155099af1b30a01a2dff), closes [#156](https://github.com/mozilla-services/autopush-rs/issues/156))
*   Basic autoendpoint extractors (#151) ([b08fdbdd](https://github.com/mozilla-services/autopush-rs/commit/b08fdbdd61edbc817835ab0fb18f304fd6fda505))

#### Bug Fixes

*   enforce VAPID `aud` (#225) ([e3963262](https://github.com/mozilla-services/autopush-rs/commit/e39632627074e475dd4c03b99e28918dc4261ec8))
*   Fix debug info setting being ignored (#219) ([35a9d4f6](https://github.com/mozilla-services/autopush-rs/commit/35a9d4f6455dc8c40928af19f32a565c4d7fbcec))
*   Check the max data size against the final message payload (#212) ([4e07ff07](https://github.com/mozilla-services/autopush-rs/commit/4e07ff07df7befd0ae29abfab335a870cb8c9dd6), closes [#203](https://github.com/mozilla-services/autopush-rs/issues/203))
*   Drop 0 TTL WebPush notifications if they aren't delivered the first time (#210) ([a28cb295](https://github.com/mozilla-services/autopush-rs/commit/a28cb2955c87125e61b3400bb539898e17e1107a))
*   Fix having extra slashes in the endpoint URL (#206) ([f943659e](https://github.com/mozilla-services/autopush-rs/commit/f943659e5a87aacaa367d818f344d881a3f77ca1))
*   Drop unknown FCM users (#197) ([068f54dd](https://github.com/mozilla-services/autopush-rs/commit/068f54ddaa00b54fb0ab8d8a12cc2251f02a31fd), closes [#173](https://github.com/mozilla-services/autopush-rs/issues/173))
*   Strip padding and double quotes from encryption and crypto-key headers (#200) ([e20fc6af](https://github.com/mozilla-services/autopush-rs/commit/e20fc6afd86d2ec18394181cedd992e2987b7858), closes [#192](https://github.com/mozilla-services/autopush-rs/issues/192))
*   Copy and upgrade parts of DynamoStorage into autoendpoint (#174) ([120a46b7](https://github.com/mozilla-services/autopush-rs/commit/120a46b75a82f1cb12ab6a93c2707f7613dc4f59), closes [#172](https://github.com/mozilla-services/autopush-rs/issues/172))
*   Use errnos from validation errors (#184) ([147aed84](https://github.com/mozilla-services/autopush-rs/commit/147aed84801ab0699fd0d9426f95a9c642a239bc))

#### Doc

*   Add a fernet_key.py script for generating Fernet keys (#218) ([aa0e9d96](https://github.com/mozilla-services/autopush-rs/commit/aa0e9d9629f7e844adbf52f44721b3bc7f0b1b13), closes [#217](https://github.com/mozilla-services/autopush-rs/issues/217))
*   Add a sample config for autopush and fix some settings (#216) ([5badbfbe](https://github.com/mozilla-services/autopush-rs/commit/5badbfbefae8b637f5062ef4f708f7016cfde29b))
*   Add a sample config for autoendpoint and normalize router settings (#214) ([3b30d694](https://github.com/mozilla-services/autopush-rs/commit/3b30d694e528a3d55117d456d81b0c05f72bdfd6))

#### Chore

*   release 1.57.0 ([fc46f768](https://github.com/mozilla-services/autopush-rs/commit/fc46f768c7fc41b0288dd6ae4fc6581558221f93))
*   Re-enable cargo-audit in CI (#221) ([e8179a85](https://github.com/mozilla-services/autopush-rs/commit/e8179a85b87f953a7fea3235315877706e26b3f7))
*   Update Docker rust to 1.45 (#193) ([9dd589ce](https://github.com/mozilla-services/autopush-rs/commit/9dd589ce449b76a7c773e28638ffb528adf283a6))



<a name="1.57.0"></a>
## 1.57.0 (2020-10-16)

Includes autoendpoint

#### Bug Fixes

*   Fix debug info setting being ignored (#219) ([35a9d4f6](https://github.com/mozilla-services/autopush-rs/commit/35a9d4f6455dc8c40928af19f32a565c4d7fbcec))
*   Check the max data size against the final message payload (#212) ([4e07ff07](https://github.com/mozilla-services/autopush-rs/commit/4e07ff07df7befd0ae29abfab335a870cb8c9dd6), closes [#203](https://github.com/mozilla-services/autopush-rs/issues/203))
*   Drop 0 TTL WebPush notifications if they aren't delivered the first time (#210) ([a28cb295](https://github.com/mozilla-services/autopush-rs/commit/a28cb2955c87125e61b3400bb539898e17e1107a))
*   Fix having extra slashes in the endpoint URL (#206) ([f943659e](https://github.com/mozilla-services/autopush-rs/commit/f943659e5a87aacaa367d818f344d881a3f77ca1))
*   Drop unknown FCM users (#197) ([068f54dd](https://github.com/mozilla-services/autopush-rs/commit/068f54ddaa00b54fb0ab8d8a12cc2251f02a31fd), closes [#173](https://github.com/mozilla-services/autopush-rs/issues/173))
*   Strip padding and double quotes from encryption and crypto-key headers (#200) ([e20fc6af](https://github.com/mozilla-services/autopush-rs/commit/e20fc6afd86d2ec18394181cedd992e2987b7858), closes [#192](https://github.com/mozilla-services/autopush-rs/issues/192))
*   Copy and upgrade parts of DynamoStorage into autoendpoint (#174) ([120a46b7](https://github.com/mozilla-services/autopush-rs/commit/120a46b75a82f1cb12ab6a93c2707f7613dc4f59), closes [#172](https://github.com/mozilla-services/autopush-rs/issues/172))
*   Use errnos from validation errors (#184) ([147aed84](https://github.com/mozilla-services/autopush-rs/commit/147aed84801ab0699fd0d9426f95a9c642a239bc))

#### Doc

*   Add a fernet_key.py script for generating Fernet keys (#218) ([aa0e9d96](https://github.com/mozilla-services/autopush-rs/commit/aa0e9d9629f7e844adbf52f44721b3bc7f0b1b13), closes [#217](https://github.com/mozilla-services/autopush-rs/issues/217))
*   Add a sample config for autopush and fix some settings (#216) ([5badbfbe](https://github.com/mozilla-services/autopush-rs/commit/5badbfbefae8b637f5062ef4f708f7016cfde29b))
*   Add a sample config for autoendpoint and normalize router settings (#214) ([3b30d694](https://github.com/mozilla-services/autopush-rs/commit/3b30d694e528a3d55117d456d81b0c05f72bdfd6))

#### Features

*   Include minimal debug info in release builds (#215) ([fd659e3e](https://github.com/mozilla-services/autopush-rs/commit/fd659e3e613114ead60fc5b28348711bc57514ac), closes [#77](https://github.com/mozilla-services/autopush-rs/issues/77))
*   Support "gcm" as an alias to "fcm" (#211) ([fd0d63d2](https://github.com/mozilla-services/autopush-rs/commit/fd0d63d2767e85e76955eab232ca49f663bf7f60), closes [#204](https://github.com/mozilla-services/autopush-rs/issues/204))
*   Add the log-check route and remove unused ApiError variants/impls (#209) ([1b0b18b3](https://github.com/mozilla-services/autopush-rs/commit/1b0b18b3b37505acedbe32bf6a8fce325986aaa1), closes [#208](https://github.com/mozilla-services/autopush-rs/issues/208))
*   Amazon Device Messaging router (#207) ([c587446a](https://github.com/mozilla-services/autopush-rs/commit/c587446a96178a2359dcb7a3dc0bd5fb33947946), closes [#165](https://github.com/mozilla-services/autopush-rs/issues/165))
*   Use autoendpoint-rs in integration tests (#205) ([31d2d19c](https://github.com/mozilla-services/autopush-rs/commit/31d2d19c971ab2a90dfe47a62b17b422aaeec33a), closes [#168](https://github.com/mozilla-services/autopush-rs/issues/168))
*   Add the unregister user route (#195) ([b4bb1636](https://github.com/mozilla-services/autopush-rs/commit/b4bb163607c302e07f779486bfb0b8ef0b7da75c), closes [#179](https://github.com/mozilla-services/autopush-rs/issues/179))
*   APNS Router (#201) ([ce51957f](https://github.com/mozilla-services/autopush-rs/commit/ce51957f0dcb1a3ead8dab3d013e43f8e976064a), closes [#164](https://github.com/mozilla-services/autopush-rs/issues/164))
*   New channel endpoint (#189) ([6cc9a7dc](https://github.com/mozilla-services/autopush-rs/commit/6cc9a7dcae95e6978cfe15c003db199ebba6fd85))
*   Update token endpoint (#188) ([bb395fb2](https://github.com/mozilla-services/autopush-rs/commit/bb395fb2bfa620016baff174ac14d57786a4197c), closes [#177](https://github.com/mozilla-services/autopush-rs/issues/177))
*   Sentry integration for autoendpoint (#196) ([674d7d2c](https://github.com/mozilla-services/autopush-rs/commit/674d7d2c1ec7d79e41e871fc0fb39dc613353e32), closes [#155](https://github.com/mozilla-services/autopush-rs/issues/155))
*   Delete message endpoint (#186) ([6a7fa492](https://github.com/mozilla-services/autopush-rs/commit/6a7fa49209bbe576840d3c40f641e854257e4417), closes [#175](https://github.com/mozilla-services/autopush-rs/issues/175))
*   User registration (#185) ([6df3e36b](https://github.com/mozilla-services/autopush-rs/commit/6df3e36bab07a68d04e7bdfff23ed327d6f98e6b), closes [#176](https://github.com/mozilla-services/autopush-rs/issues/176))
*   Route notifications to FCM (Android) (#171) ([d9a0d9d7](https://github.com/mozilla-services/autopush-rs/commit/d9a0d9d7d1bd79fc3cd786e4a27a18bca4ff3eec), closes [#162](https://github.com/mozilla-services/autopush-rs/issues/162))
*   Route notifications to autopush connection servers (#167) ([e73dff17](https://github.com/mozilla-services/autopush-rs/commit/e73dff17e9a9909743fd57bbd9d6769789a455e1), closes [#161](https://github.com/mozilla-services/autopush-rs/issues/161))
*   Return detailed autoendpoint errors (#170) ([91d483ab](https://github.com/mozilla-services/autopush-rs/commit/91d483ab5e357ad6c50dc7669a9d12cd3f1914a7), closes [#159](https://github.com/mozilla-services/autopush-rs/issues/159))
*   Record the encoding in a metric if there is an encrypted payload (#166) ([8451d3f9](https://github.com/mozilla-services/autopush-rs/commit/8451d3f9790a6633107cd5473768c1dd53602df1))
*   Validate autoendpoint JWT tokens (#154) ([04fee7f9](https://github.com/mozilla-services/autopush-rs/commit/04fee7f9cbb7fc984d34953c05f34f7a732310d4), closes [#103](https://github.com/mozilla-services/autopush-rs/issues/103))
*   Validate user subscription data in autoendpoint (#160) ([8efa42c8](https://github.com/mozilla-services/autopush-rs/commit/8efa42c8885865ae1392155099af1b30a01a2dff), closes [#156](https://github.com/mozilla-services/autopush-rs/issues/156))
*   Basic autoendpoint extractors (#151) ([b08fdbdd](https://github.com/mozilla-services/autopush-rs/commit/b08fdbdd61edbc817835ab0fb18f304fd6fda505))

#### Chore

*   Re-enable cargo-audit in CI (#221) ([e8179a85](https://github.com/mozilla-services/autopush-rs/commit/e8179a85b87f953a7fea3235315877706e26b3f7))
*   Update Docker rust to 1.45 (#193) ([9dd589ce](https://github.com/mozilla-services/autopush-rs/commit/9dd589ce449b76a7c773e28638ffb528adf283a6))



<a name="1.55.0"></a>
## 1.55.0 (2020-04-10)


#### Chore

*   add a badge for the matrix channel ([12e3aa07](https://github.com/mozilla-services/autopush-rs/commit/12e3aa07a392f35afe41c9976114e583755ca308))
*   update deps ([82cc33ee](https://github.com/mozilla-services/autopush-rs/commit/82cc33ee78c585ee658edf17eab1ddc3e06d9c1e), closes [#131](https://github.com/mozilla-services/autopush-rs/issues/131))
*   add pr template ([9a083f42](https://github.com/mozilla-services/autopush-rs/commit/9a083f42b2d9aa9b7c108c1dc98e8d70a48cd777))
*   force cargo install to use cargo.lock ([2bdcd004](https://github.com/mozilla-services/autopush-rs/commit/2bdcd0041187333f085a0c07a0cffafd9e9d53e2))

#### Test

*   ensure sane logging levels ([e68a060d](https://github.com/mozilla-services/autopush-rs/commit/e68a060d3ee92540ef8d954bd49243e45dbf8da2), closes [#117](https://github.com/mozilla-services/autopush-rs/issues/117))

#### Bug Fixes

*   double amount of allowed headers ([67033818](https://github.com/mozilla-services/autopush-rs/commit/6703381888dda87bff29f5f3436b144143bf051e), closes [#136](https://github.com/mozilla-services/autopush-rs/issues/136))



<a name="1.54.4"></a>
## 1.54.4 (2019-10-25)


#### Chore

*   switch to debian buster ([322ee852](https://github.com/mozilla-services/autopush-rs/commit/322ee852a868cb4aa80aae4b2a3bc5e5ab9d177f), closes [#124](https://github.com/mozilla-services/autopush-rs/issues/124))



<a name="1.54.3"></a>
## 1.54.3 (2019-10-11)


#### Bug Fixes

*   report Megaphone errors to Sentry ([0b6eed61](https://github.com/mozilla-services/autopush-rs/commit/0b6eed619a45ff0dc2ea2de4907a3762d00749a7))



<a name="1.54.2"></a>
## 1.54.2 (2019-09-19)


#### Bug Fixes

*   restore the log crate ([8d28c86f](https://github.com/mozilla-services/autopush-rs/commit/8d28c86f1ab02118f0ef2d047ffe50ab9fa76cf9), closes [#115](https://github.com/mozilla-services/autopush-rs/issues/115))



<a name="1.54.1"></a>
## 1.54.1 (2019-08-16)


#### Chore

*   update to latest rust/debian buster ([9b089fb4](https://github.com/mozilla-services/autopush-rs/commit/9b089fb49d84ac38bbbd069faae4b54abea9006f), closes [#110](https://github.com/mozilla-services/autopush-rs/issues/110))



<a name="1.54.0"></a>
## 1.54.0 (2019-08-06)


#### Refactor

*   simplify srv usage ([1086de7b](https://github.com/mozilla-services/autopush-rs/commit/1086de7bff281e7d428d5af989cb6e4e0f55a85c))
*   split connection node from common for later endpoint ([e523d739](https://github.com/mozilla-services/autopush-rs/commit/e523d739ee4377c99c38d30bd6f8ecb582bcbc45), closes [#99](https://github.com/mozilla-services/autopush-rs/issues/99))

#### Features

*   Perform cargo audit ([22766546](https://github.com/mozilla-services/autopush-rs/commit/227665466184a33a333b8685c077bd81742622bb), closes [#107](https://github.com/mozilla-services/autopush-rs/issues/107))

#### Chore

*   force update smallvec 0.6 & crossbeam-epoch (memoffset) ([abbd6257](https://github.com/mozilla-services/autopush-rs/commit/abbd6257e286004582ddd826277f3efbf83679a4), closes [#110](https://github.com/mozilla-services/autopush-rs/issues/110))
*   adapt to new woothee/tungstenite ([906cc2c5](https://github.com/mozilla-services/autopush-rs/commit/906cc2c57faecacc88ac317ecf83ab0f83e86470), closes [#110](https://github.com/mozilla-services/autopush-rs/issues/110))
*   update deps minus hyper 0.12 ([eaaccc09](https://github.com/mozilla-services/autopush-rs/commit/eaaccc09c57c80a3b0211c36b6e4e581945fc620))



<a name="1.53.1"></a>
## 1.53.1 (2019-02-14)


#### Features

*   retry all dynamodb errors correctly, hide invalid chids ([58152230](https://github.com/mozilla-services/autopush-rs/commit/581522309725fbfb709b54710bce249adcceedf4), closes [#95](https://github.com/mozilla-services/autopush-rs/issues/95))

#### Chore

*   update dependencies ([b4e159d4](https://github.com/mozilla-services/autopush-rs/commit/b4e159d41b6b0e4876cb1a1606b4b2ddc6687b8a))



<a name="1.53.0"></a>
## 1.53.0 (2019-01-19)


#### Bug Fixes

*   capture additional errors that shouldn't be reported to Sentry ([3f5f24f6](https://github.com/mozilla-services/autopush-rs/commit/3f5f24f6cc08f8ef04aa07116660e526f1ef0d8b), closes [#87](https://github.com/mozilla-services/autopush-rs/issues/87))
*   return correct not found for disconnected client and fix tests ([5d6d29df](https://github.com/mozilla-services/autopush-rs/commit/5d6d29dfc8c955ef25d810eecb977c992a5f92b4), closes [#89](https://github.com/mozilla-services/autopush-rs/issues/89))

#### Chore

*   some cleanup after cargo fixes ([6c118212](https://github.com/mozilla-services/autopush-rs/commit/6c118212f59edfa99370345752ab1fc5f510bc7f))
*   cargo fix --edition-idioms ([909965f3](https://github.com/mozilla-services/autopush-rs/commit/909965f32580986a5146f1125d05f8516487c4ee))
*   cargo fix --edition ([00592dd6](https://github.com/mozilla-services/autopush-rs/commit/00592dd61b62ea6067349ddb2717b261f3fb72e5))
*   update dependencies ([9244c7fd](https://github.com/mozilla-services/autopush-rs/commit/9244c7fddb9105829f424816a87352a6af4f86af), closes [#91](https://github.com/mozilla-services/autopush-rs/issues/91))
*   cargo fmt (1.31.0) ([801b1e13](https://github.com/mozilla-services/autopush-rs/commit/801b1e13455a7761859151f77f6aa55a9bb911ff))
*   update Cargo.lock to bump requests to >= 2.20.0 ([5e837376](https://github.com/mozilla-services/autopush-rs/commit/5e837376e4fff93b7cb841ecb37f9cf4bfb4938c), closes [#84](https://github.com/mozilla-services/autopush-rs/issues/84))



<a name="1.52.0"></a>
## 1.52.0 (2018-10-12)


#### Bug Fixes

*   quick workaround for rusoto's hyper upgrade ([06eff39b](https://github.com/mozilla-services/autopush-rs/commit/06eff39b845167fa331b23e1a8d841c394772045))

#### Chore

*   upgrade to latest rust ([4230d0cf](https://github.com/mozilla-services/autopush-rs/commit/4230d0cfc93ecd41d8500ed3186e0c23e861e2ab))
*   upgrade test_integrations deps ([93a5df21](https://github.com/mozilla-services/autopush-rs/commit/93a5df21896dd8ae5084f62cb888b20d270200a5))
*   upgrade dependencies minus hyper and woothee ([790ebdbb](https://github.com/mozilla-services/autopush-rs/commit/790ebdbb921014edbcf32525fa25ccc3cc41050b))



<a name="1.51.2"></a>
## 1.51.2 (2018-09-14)


#### Features

*   clean-up sentry error reporting and reduce spurious reporting ([f0bb4e0e](https://github.com/mozilla-services/autopush-rs/commit/f0bb4e0e7c517e93f17686c12655e88761306caa), closes [#71](https://github.com/mozilla-services/autopush-rs/issues/71))

#### Bug Fixes

*   ignore invalid state transitions ([464e5f93](https://github.com/mozilla-services/autopush-rs/commit/464e5f93ff0e61ba1a04cd52c32a05e7cac11c9c))
*   don't include ports for schemes they aren't needed for ([4698f06f](https://github.com/mozilla-services/autopush-rs/commit/4698f06fc353a18d7f7cdf22073648730515de66))
*   use latest sentry ([79b0fedb](https://github.com/mozilla-services/autopush-rs/commit/79b0fedb109b76dd005ce355f933dc957bd4e4e6))



<a name="1.51.1"></a>
## 1.51.1 (2018-08-30)


#### Features

*   fix sentry test and add release data to sentry errors ([8e59c674](https://github.com/mozilla-services/autopush-rs/commit/8e59c674bfdfbd19eb4cacbc8477d49f84b02fd3), closes [#66](https://github.com/mozilla-services/autopush-rs/issues/66))



<a name="1.51.0"></a>
## 1.51.0 (2018-08-30)


#### Features

*   upgrade sentry to 0.8.0 and log out errors ([3ec1d3c8](https://github.com/mozilla-services/autopush-rs/commit/3ec1d3c87218e9cfe5aafe6bbfb0363fc89da526), closes [#5](https://github.com/mozilla-services/autopush-rs/issues/5))
*   return broadcast errors for invalid broadcast id's ([ee7cb913](https://github.com/mozilla-services/autopush-rs/commit/ee7cb913ea61a4000147b3ac0a8346c1709bb7b0), closes [#59](https://github.com/mozilla-services/autopush-rs/issues/59))
*   notify other nodes if user has reconnected for missed messages ([10152fcb](https://github.com/mozilla-services/autopush-rs/commit/10152fcb44696a8e99091d102624c4a9b33749c3), closes [#58](https://github.com/mozilla-services/autopush-rs/issues/58))

#### Chore

*   rustfmt update ([583d07a7](https://github.com/mozilla-services/autopush-rs/commit/583d07a7ff15c544811909aadb7b9727fb180691))



<a name="1.50.0"></a>
## 1.50.0 (2018-08-16)


#### Bug Fixes

*   don't render topic/timestamp to the ua ([8aa47af4](https://github.com/mozilla-services/autopush-rs/commit/8aa47af440bc962aadc7ca0092789e8936574363))



<a name="1.49.7"></a>
## 1.49.7 (2018-08-07)


#### Features

*   support webpush API pings ([2366efe4](https://github.com/mozilla-services/autopush-rs/commit/2366efe4e1132aec66e0992e98616084937c49e7), closes [#55](https://github.com/mozilla-services/autopush-rs/issues/55))
*   log out disconnect reason with session statistics ([1109aa2f](https://github.com/mozilla-services/autopush-rs/commit/1109aa2fe194a94771fc8a0df049d228bc6cf728))



<a name="1.49.6"></a>
## 1.49.6 (2018-08-02)


#### Chore

*   include CIRCLE_TAG in the cache key ([d85cee70](https://github.com/mozilla-services/autopush-rs/commit/d85cee709558b1f80ba4d2cce91ad17b49627acb))



<a name="1.49.5"></a>
## 1.49.5 (2018-08-02)


#### Bug Fixes

*   remove the host tag/log field ([7425749c](https://github.com/mozilla-services/autopush-rs/commit/7425749c4503b2e4858a67c8e4e2d4c16c9937f8), closes [#41](https://github.com/mozilla-services/autopush-rs/issues/41))



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
