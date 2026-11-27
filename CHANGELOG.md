# Changelog

## [1.4.1](https://github.com/MattJColes/herdr-web-browser/compare/v1.4.0...v1.4.1) (2026-11-27)


### Bug Fixes

* **deps:** Bump the cargo-minor-and-patch group with 2 updates ([c48c41f](https://github.com/MattJColes/herdr-web-browser/commit/c48c41f858c6cd0d1318ff8dba84df954c5bc38d))
* **deps:** Bump the cargo-minor-and-patch group with 2 updates ([8d7866a](https://github.com/MattJColes/herdr-web-browser/commit/8d7866a3e49d75755fa80d9d71e12ba473e50ad8))
* open the + menu browser beside an agent in its own space ([9d406d3](https://github.com/MattJColes/herdr-web-browser/commit/9d406d3ed0ac7f2855d12e12d9ad79e87869523b))
* open the + menu browser beside an agent in its own space ([1b29603](https://github.com/MattJColes/herdr-web-browser/commit/1b296035c9d38169ac197ec0b3105ea8616a3ef2))
* run the live view in nushell panes ([0a6c6a2](https://github.com/MattJColes/herdr-web-browser/commit/0a6c6a28eaffba8a24ccf4f6d40a42748f563e5a))
* run the live view in nushell panes ([15ab5a9](https://github.com/MattJColes/herdr-web-browser/commit/15ab5a9f44b6df4e2fb215e0a0fd0f43696059c2))
* size herdr desktop cells from the configured font on macOS ([cacc239](https://github.com/MattJColes/herdr-web-browser/commit/cacc239cbfbcec8bc17fdb81868653eaeeb95119))
* size herdr desktop cells from the configured font on macOS ([5dfd321](https://github.com/MattJColes/herdr-web-browser/commit/5dfd321c62a9b12f8a86cdbed5a8cd45d2fe6122))
* stream full-resolution live view frames from headless Chrome on HiDPI screens ([ebd7477](https://github.com/MattJColes/herdr-web-browser/commit/ebd7477b76ce7d35d7fb1e0ba56b4fdd36677463))
* stream full-resolution live view frames from headless Chrome on HiDPI screens ([012b4cd](https://github.com/MattJColes/herdr-web-browser/commit/012b4cdf9f6adb5f2fc46b36216836e1c38809c9))

## [1.4.0](https://github.com/MattJColes/herdr-web-browser/compare/v1.3.3...v1.4.0) (2026-11-23)


### Features

* add ~/.local/bin to the user PATH on Windows in install-cli ([828515e](https://github.com/MattJColes/herdr-web-browser/commit/828515ec87c4a924b96fe1388e2658918b11dbf3))
* find Chrome, Edge, Brave and Chromium on Windows and download Chrome for Testing win64 ([d00aa48](https://github.com/MattJColes/herdr-web-browser/commit/d00aa4857c9a3aeb1f34b6fb48e16e8bd100d050))
* native Windows support ([2f59160](https://github.com/MattJColes/herdr-web-browser/commit/2f5916006fa1fac0666457778bf38218802a656e))
* register the native messaging host in the Windows registry ([9d17db9](https://github.com/MattJColes/herdr-web-browser/commit/9d17db9c6865ac2b2ae50c9eb483914ee81a9f7c))
* run plugin actions without /bin/sh and list windows in platforms ([6047d7a](https://github.com/MattJColes/herdr-web-browser/commit/6047d7ad912fd0e9ede080db9b1fec6f7a572cc0))
* sharper live view in Windows Terminal with octant characters ([a50bf16](https://github.com/MattJColes/herdr-web-browser/commit/a50bf161242b773af5ccc67bc60e9513968e881e))


### Bug Fixes

* bind the herdr desktop browser menu to ctrl-alt-b on Windows ([c0a078d](https://github.com/MattJColes/herdr-web-browser/commit/c0a078dff42dc81b01ae75278ef19c9897768094))
* bring headed tab to front before click and type ([273c229](https://github.com/MattJColes/herdr-web-browser/commit/273c22979b176c207355a43e5938fe66949d1f3e))
* bring headed tab to front before click and type ([287f7e9](https://github.com/MattJColes/herdr-web-browser/commit/287f7e9dc80aa4822dcaa2705d3e8f9b5379a5ab))
* build Windows paths from components so they print with one separator ([c5b27a3](https://github.com/MattJColes/herdr-web-browser/commit/c5b27a3de489d3a802f9827be72c4a6bf05c6b9c))
* compile on Windows and use Windows home, state and config paths ([16cfc9a](https://github.com/MattJColes/herdr-web-browser/commit/16cfc9a730cf3e61ece2ec896d331321354f6650))
* half-block live view in herdr desktop on Windows on ARM ([f3656f6](https://github.com/MattJColes/herdr-web-browser/commit/f3656f68c890f6234f71b3c673b25b174a674284))
* keep Edge from signing in to the Windows account and syncing ([8fd9e42](https://github.com/MattJColes/herdr-web-browser/commit/8fd9e4229db535137f41ddbbb03bb02ded0c7154))
* let cargo rebuild while herdr panes run the Windows exe ([f2572c6](https://github.com/MattJColes/herdr-web-browser/commit/f2572c6f2658458906c6d4fd38e8e69e2d088550))
* quote the live view command for PowerShell in herdr panes on Windows ([382b910](https://github.com/MattJColes/herdr-web-browser/commit/382b9103cac58572c035ad5856e1a2012f5ef6db))
* stop Chrome holding the caller's output pipe open on Windows ([00bb5b3](https://github.com/MattJColes/herdr-web-browser/commit/00bb5b37780c12388fa096a8ac1240eb52f5273f))
* stop install-cli from changing agent instructions ([7090688](https://github.com/MattJColes/herdr-web-browser/commit/7090688a9bff4f0993495aad44e16d86f2ccd21e))
* stop install-cli from changing agent instructions ([06b8a22](https://github.com/MattJColes/herdr-web-browser/commit/06b8a225884352d09b765a88168450f78581c7a0)), closes [#30](https://github.com/MattJColes/herdr-web-browser/issues/30)

## [1.3.3](https://github.com/MattJColes/herdr-web-browser/compare/v1.3.2...v1.3.3) (2026-11-18)


### Bug Fixes

* **deps:** Bump resvg from 0.49.0 to 0.50.0 ([3f17857](https://github.com/MattJColes/herdr-web-browser/commit/3f17857608017dc0fd034ad20c4da1886565641f))
* **deps:** Bump resvg from 0.49.0 to 0.50.0 ([d2aaa58](https://github.com/MattJColes/herdr-web-browser/commit/d2aaa58a8eb6a0c2319d75c1f967cb966e90ac47))
* **view:** size herdr desktop's live view from real cell metrics ([1a578d1](https://github.com/MattJColes/herdr-web-browser/commit/1a578d1bcbc579681d669038a63e984637b2c19a))
* **view:** size herdr desktop's live view from real cell metrics ([5b25089](https://github.com/MattJColes/herdr-web-browser/commit/5b2508993fe40f7e08d504130de61cb004f34300))

## [1.3.2](https://github.com/MattJColes/herdr-web-browser/compare/v1.3.1...v1.3.2) (2026-11-11)


### Bug Fixes

* release patch versions for dependency bumps ([15a5f7e](https://github.com/MattJColes/herdr-web-browser/commit/15a5f7e93ee9afd155610fbdd86fa4c82f36c130))
* release patch versions for dependency bumps ([f24027a](https://github.com/MattJColes/herdr-web-browser/commit/f24027a1d784c66617cba16d294ee2b3462c8a0d))

## [1.3.1](https://github.com/MattJColes/herdr-web-browser/compare/v1.3.0...v1.3.1) (2026-11-04)


### Bug Fixes

* **desktop:** re-point desktop mods at the running build ([b2c7ffd](https://github.com/MattJColes/herdr-web-browser/commit/b2c7ffd95229c38e66c6214f51e021a3ec2b2d4c))
* **desktop:** Re-point desktop mods at the running build ([3751fe1](https://github.com/MattJColes/herdr-web-browser/commit/3751fe1c4d6e03557a0da4ab70803d327eab0377))

## [1.3.0](https://github.com/MattJColes/herdr-web-browser/compare/v1.2.0...v1.3.0) (2026-11-03)


### Features

* Install agents and Chrome for Testing on startup ([89d5100](https://github.com/MattJColes/herdr-web-browser/commit/89d5100fd47f391360522e438766740e0d26880f))
* set up desktop, agents and Chrome for Testing by default ([0506466](https://github.com/MattJColes/herdr-web-browser/commit/0506466708b0be9f031b4378958b8cd4eaaffcd3))

## [1.2.0](https://github.com/MattJColes/herdr-web-browser/compare/v1.1.0...v1.2.0) (2026-11-03)


### Features

* **desktop:** set up herdr desktop and its menu shortcut by default ([ab58e62](https://github.com/MattJColes/herdr-web-browser/commit/ab58e62b8ad43c1044334453256d953faca94c78))
* **desktop:** set up herdr desktop and its menu shortcut by default ([8905262](https://github.com/MattJColes/herdr-web-browser/commit/8905262b3ac0e95adb2b4439f5f83e2a96caab24))

## [1.1.0](https://github.com/MattJColes/herdr-web-browser/compare/v1.0.0...v1.1.0) (2026-11-03)


### Features

* uninstall command; README update and uninstall steps ([d1783f5](https://github.com/MattJColes/herdr-web-browser/commit/d1783f5a6a5b6c6aac8d7af2f286ff0b94866e3d))
* uninstall command; README update and uninstall steps ([e16c6a3](https://github.com/MattJColes/herdr-web-browser/commit/e16c6a30672e489ab118d4e0a368b55b76e138e8))


### Performance Improvements

* **view:** raw RGBA frames with changed-region updates on herdr desktop ([f0236ee](https://github.com/MattJColes/herdr-web-browser/commit/f0236ee6f346049bf5ab53b0c785072f6789e466))
* **view:** raw RGBA frames with changed-region updates on herdr desktop ([351b020](https://github.com/MattJColes/herdr-web-browser/commit/351b020eccbfc2ba0913c157bdb11039c6e9e4ef))

## [1.0.0](https://github.com/MattJColes/herdr-web-browser/compare/v0.1.0...v1.0.0) (2026-11-03)


### Features

* agent setup, console/network debugging, Chrome sign-in import ([48305dd](https://github.com/MattJColes/herdr-web-browser/commit/48305dde8ed75f92d4b272504b3a9be32ae74d7a))
* **desktop:** open Browser from the + menu beside the agent ([c1da26c](https://github.com/MattJColes/herdr-web-browser/commit/c1da26c44cf4c3bebfdb9ac2ed0f494be6fc751b))
* ready for herdr's plugin marketplace; ctrl-click localhost links ([fdad62d](https://github.com/MattJColes/herdr-web-browser/commit/fdad62d04a2ba7eb62a8ae6fe3834bc419b01acb))
* start the browser headless by default ([89d2679](https://github.com/MattJColes/herdr-web-browser/commit/89d26799e49d38613089d65ab3ffb2786e14ec14))
* **view:** tools beside the address bar, word/all selection, balanced page ([7d9a3f6](https://github.com/MattJColes/herdr-web-browser/commit/7d9a3f669985cd23d3da2a1a5d0a928520ff1b26))


### Bug Fixes

* **deps:** adapt to png 0.18 ([998788b](https://github.com/MattJColes/herdr-web-browser/commit/998788b21a8cf3e9fba187bfcfb98648a3bd3eed))
* **deps:** move RustCrypto crates to aes 0.9 / pbkdf2 0.13 together ([0d93989](https://github.com/MattJColes/herdr-web-browser/commit/0d939895f4a0962906159bc7acf3af5fa866cd74))
* **desktop:** settings dialog search takes typing ([aded702](https://github.com/MattJColes/herdr-web-browser/commit/aded70213363bc74435d16f5a0b6158c1a0812e2))
* only show a Chrome window when asked, keep headless pages painting ([365ac2c](https://github.com/MattJColes/herdr-web-browser/commit/365ac2c8ea2567f82fa2da8c1e910ed1fca94546))
* read the Chrome profile you actually use when importing logins ([558c566](https://github.com/MattJColes/herdr-web-browser/commit/558c56641b20ea7334488c3227d09b21787aa289))
* run Chrome for Testing so the hidden browser can't take over Chrome ([2b917ab](https://github.com/MattJColes/herdr-web-browser/commit/2b917abe83d4caa5c510e5807f8033f7e26b9321))


### Continuous Integration

* release with Release Please, starting at 1.0.0 ([25a8d4d](https://github.com/MattJColes/herdr-web-browser/commit/25a8d4d87c4120643eb2619f490204e68682d957))
