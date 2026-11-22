# Changelog

All notable changes to SwiftWin. Versions follow [Semantic Versioning](https://semver.org); downloads are
on the [Releases page](https://github.com/Dadon-11/SwiftWin-Studio/releases).

## [0.1.2] - 2026-11-22

### Fixed
- "Set up this PC" showed Apple's USB service (iTunes) as "TO DO" when iTunes had put its driver in the
  32-bit "Common Files" folder. SwiftWin now looks in all of them.

### Added
- **iTunes is already installed: choose its folder**: if SwiftWin still can't find Apple's USB service, you
  can pick the folder yourself.
- The installer has a SwiftWin banner, a license page, and copyright and homepage details.

## [0.1.1] - 2026-11-15

### Fixed
- The name field in Account settings didn't accept typing.

## [0.1.0] - 2026-11-15

First public release.

- Build, sign, install and run SwiftUI apps on a real iPhone over USB from Windows, with no Mac.
- SwiftWin Studio: file tree, code editor, app logs, crash reports and a live phone mirror you can tap,
  swipe and type into.
- AI mode and an MCP server for Claude Code and OpenAI Codex.
- Inspo: turn an app on your phone into a palette, type scale and spacing guide.
- Set up this PC, `swiftwin doctor`, free Apple ID or paid team signing, and phone control with
  WebDriverAgent.
- Optional account (Google or GitHub) for cloud TestFlight builds and uploading projects to GitHub.
- Automatic, signed updates from GitHub Releases.

[0.1.2]: https://github.com/Dadon-11/SwiftWin-Studio/releases/tag/v0.1.2
[0.1.1]: https://github.com/Dadon-11/SwiftWin-Studio/releases/tag/v0.1.1
[0.1.0]: https://github.com/Dadon-11/SwiftWin-Studio/releases/tag/v0.1.0
