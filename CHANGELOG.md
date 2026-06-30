# Changelog

## [0.2.1](https://github.com/lawther/claude_status_line/compare/claude_status_line-v0.2.0...claude_status_line-v0.2.1) (2026-06-30)


### Bug Fixes

* trigger release-please PR ([dcb6f97](https://github.com/lawther/claude_status_line/commit/dcb6f97b4327ddc6817ae8a4fecf83e33cc52b84))

## [0.2.0](https://github.com/lawther/claude_status_line/compare/claude_status_line-v0.1.0...claude_status_line-v0.2.0) (2026-06-30)


### Features

* add --install and --help flags, drop jq dependency ([946ef3a](https://github.com/lawther/claude_status_line/commit/946ef3ab8cd3f0210ca9f256d1b09758f9f56bd8))
* add -q/--quiet flag to --install ([5811427](https://github.com/lawther/claude_status_line/commit/581142736a06fbdccccbff126c693119e93696d4))
* add dynamic bar width (15→5 cells) to compression sequence ([88f2e4f](https://github.com/lawther/claude_status_line/commit/88f2e4f27006116385991d681aaa14edd61440f6))
* replace binary compact mode with dynamic compression levels ([be6511c](https://github.com/lawther/claude_status_line/commit/be6511c1aa4eef735eafc4a9de0b77587bd75b59))
* show 5h quota times as absolute clock times ([15b1978](https://github.com/lawther/claude_status_line/commit/15b19781f79176684c9e1a293ea9bf53b3537378))
* update pace symbols and add bars to quota displays ([d51f866](https://github.com/lawther/claude_status_line/commit/d51f8669a03d1f053917ce495b83b32da73a5cea))


### Bug Fixes

* account for Claude Code's 4-space padding and font-missing chars ([c2a9315](https://github.com/lawther/claude_status_line/commit/c2a93155c78012dfaf76c0ef6a22dc8b97cff2f1))
* count U+FE0F emoji presentation selector as 1 column wide ([d1cfe3c](https://github.com/lawther/claude_status_line/commit/d1cfe3c8eed5e4039680db1648d3353ce5b6dbeb))
* restore pace symbols and widen COLUMNS safety margin to 9 ([b7ad0f1](https://github.com/lawther/claude_status_line/commit/b7ad0f1f8623ef1d99656ec8aea24c8da4943bc7))
* update yellow zone to 90% ([9097b78](https://github.com/lawther/claude_status_line/commit/9097b785b1d6b52d76bb8abda8f3223073e75caf))
