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



